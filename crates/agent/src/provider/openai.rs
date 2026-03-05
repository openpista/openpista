//! OpenAI-compatible provider implementation.

use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
        ChatCompletionRequestUserMessageArgs, ChatCompletionTool, ChatCompletionToolType,
        CreateChatCompletionRequestArgs, FinishReason, FunctionObjectArgs,
    },
};
use async_trait::async_trait;
use proto::{LlmError, ToolCall, ToolDefinition};
use serde_json::Value;
use tracing::debug;

use super::{ChatMessage, ChatRequest, ChatResponse, LlmProvider, TokenUsage};

/// OpenAI-compatible provider (works with OpenAI, together.ai, etc.)
///
/// **Note:** For reasoning models (o1, o3, o4-mini, codex), use the
/// [`ResponsesApiProvider`](super::responses::ResponsesApiProvider) which
/// supports the `reasoning.effort` parameter. This ChatCompletions-based
/// provider is used for non-reasoning models and third-party endpoints
/// (Together, Ollama, etc.) that don't support the reasoning parameter.
pub struct OpenAiProvider {
    client: Client<OpenAIConfig>,
}

impl OpenAiProvider {
    /// Creates an OpenAI provider using the default API base URL.
    pub fn new(api_key: impl Into<String>, _model: impl Into<String>) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let client = Client::with_config(config);
        Self { client }
    }

    /// Creates an OpenAI provider with a custom API base URL.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        _model: impl Into<String>,
    ) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(base_url);
        let client = Client::with_config(config);
        Self { client }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        // Convert messages
        let messages: Vec<ChatCompletionRequestMessage> = req
            .messages
            .iter()
            .map(convert_message)
            .collect::<Result<_, _>>()?;

        // Convert tools
        let tools: Vec<ChatCompletionTool> = req
            .tools
            .iter()
            .map(convert_tool)
            .collect::<Result<_, _>>()?;

        let mut builder = CreateChatCompletionRequestArgs::default();
        builder.model(&req.model).messages(messages);

        if !tools.is_empty() {
            builder.tools(tools);
        }

        let request = builder
            .build()
            .map_err(|e| LlmError::Serialization(e.to_string()))?;

        debug!(
            model = %req.model,
            messages = %req.messages.len(),
            tools = %req.tools.len(),
            "Sending request to OpenAI"
        );

        let response = self.client.chat().create(request).await.map_err(|e| {
            let msg = e.to_string();
            debug!(error = %msg, "OpenAI API error");
            let hint = if msg.contains("does not exist") || msg.contains("model_not_found") {
                " Try /model to select a different model."
            } else if msg.to_lowercase().contains("billing") || msg.to_lowercase().contains("quota")
            {
                " Check your OpenAI billing at https://platform.openai.com."
            } else {
                ""
            };
            LlmError::Api(format!("{msg}{hint}"))
        })?;

        let usage = TokenUsage {
            prompt_tokens: response.usage.as_ref().map_or(0, |u| u.prompt_tokens),
            completion_tokens: response.usage.as_ref().map_or(0, |u| u.completion_tokens),
        };
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::InvalidResponse("No choices in response".into()))?;
        match choice.finish_reason {
            Some(FinishReason::ToolCalls) => {
                let tool_calls = choice
                    .message
                    .tool_calls
                    .unwrap_or_default()
                    .into_iter()
                    .map(|tc| {
                        let args = parse_tool_arguments(&tc.function.arguments);
                        ToolCall {
                            id: tc.id,
                            name: tc.function.name,
                            arguments: args,
                        }
                    })
                    .collect();
                Ok(ChatResponse::ToolCalls(tool_calls, usage))
            }
            _ => {
                let text = choice.message.content.unwrap_or_default();
                Ok(ChatResponse::Text(text, usage))
            }
        }
    }
}

/// Converts internal chat message into OpenAI request format.
fn convert_message(m: &ChatMessage) -> Result<ChatCompletionRequestMessage, LlmError> {
    match m.role {
        proto::Role::System => Ok(ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(m.content.clone())
                .build()
                .map_err(|e| LlmError::Serialization(e.to_string()))?,
        )),
        proto::Role::User => Ok(ChatCompletionRequestMessage::User(
            ChatCompletionRequestUserMessageArgs::default()
                .content(m.content.clone())
                .build()
                .map_err(|e| LlmError::Serialization(e.to_string()))?,
        )),
        proto::Role::Assistant => {
            let mut builder = ChatCompletionRequestAssistantMessageArgs::default();
            builder.content(m.content.clone());

            if let Some(tool_calls) = &m.tool_calls {
                let tc: Vec<async_openai::types::ChatCompletionMessageToolCall> = tool_calls
                    .iter()
                    .map(|tc| async_openai::types::ChatCompletionMessageToolCall {
                        id: tc.id.clone(),
                        r#type: ChatCompletionToolType::Function,
                        function: async_openai::types::FunctionCall {
                            name: tc.name.clone(),
                            arguments: tc.arguments.to_string(),
                        },
                    })
                    .collect();
                builder.tool_calls(tc);
            }

            Ok(ChatCompletionRequestMessage::Assistant(
                builder
                    .build()
                    .map_err(|e| LlmError::Serialization(e.to_string()))?,
            ))
        }
        proto::Role::Tool => {
            let call_id = m
                .tool_call_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            Ok(ChatCompletionRequestMessage::Tool(
                ChatCompletionRequestToolMessageArgs::default()
                    .tool_call_id(call_id)
                    .content(m.content.clone())
                    .build()
                    .map_err(|e| LlmError::Serialization(e.to_string()))?,
            ))
        }
    }
}

/// Converts internal tool schema into OpenAI function-tool declaration.
fn convert_tool(t: &ToolDefinition) -> Result<ChatCompletionTool, LlmError> {
    Ok(ChatCompletionTool {
        r#type: ChatCompletionToolType::Function,
        function: FunctionObjectArgs::default()
            .name(t.name.clone())
            .description(t.description.clone())
            .parameters(t.parameters.clone())
            .build()
            .map_err(|e| LlmError::Serialization(e.to_string()))?,
    })
}

/// Parses tool call argument JSON with empty-object fallback.
fn parse_tool_arguments(arguments: &str) -> Value {
    serde_json::from_str(arguments).unwrap_or(Value::Object(Default::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_message_supports_all_roles() {
        let system = convert_message(&ChatMessage::system("sys")).expect("system");
        assert!(matches!(system, ChatCompletionRequestMessage::System(_)));

        let user = convert_message(&ChatMessage::user("hello")).expect("user");
        assert!(matches!(user, ChatCompletionRequestMessage::User(_)));

        let assistant = convert_message(&ChatMessage::assistant("done")).expect("assistant");
        assert!(matches!(
            assistant,
            ChatCompletionRequestMessage::Assistant(_)
        ));

        let tool = convert_message(&ChatMessage::tool_result("id", "echo", "ok")).expect("tool");
        assert!(matches!(tool, ChatCompletionRequestMessage::Tool(_)));
    }

    #[test]
    fn convert_message_assistant_with_tool_calls_includes_calls() {
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = Some(vec![ToolCall {
            id: "tc1".to_string(),
            name: "system.run".to_string(),
            arguments: serde_json::json!({"command":"echo hi"}),
        }]);

        let converted = convert_message(&assistant).expect("assistant with tool calls");
        let ChatCompletionRequestMessage::Assistant(msg) = converted else {
            panic!("expected assistant message");
        };
        let calls = msg.tool_calls.expect("tool calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "tc1");
        assert_eq!(calls[0].function.name, "system.run");
    }

    #[test]
    fn convert_tool_builds_function_tool_schema() {
        let def = ToolDefinition::new(
            "system.run",
            "Run shell command",
            serde_json::json!({"type":"object"}),
        );
        let converted = convert_tool(&def).expect("tool conversion");
        assert_eq!(converted.r#type, ChatCompletionToolType::Function);
        assert_eq!(converted.function.name, "system.run");
        assert_eq!(
            converted.function.description.as_deref(),
            Some("Run shell command")
        );
    }

    #[test]
    fn parse_tool_arguments_handles_valid_and_invalid_json() {
        let valid = parse_tool_arguments(r#"{"x":1}"#);
        assert_eq!(valid["x"], 1);

        let invalid = parse_tool_arguments("{invalid");
        assert!(invalid.is_object());
        assert_eq!(invalid.as_object().expect("object").len(), 0);
    }

    #[test]
    fn provider_builders_construct_provider_instances() {
        let _provider = OpenAiProvider::new("k", "m");
        let _provider = OpenAiProvider::with_base_url("k", "https://example.com/v1", "m");
    }

    #[test]
    fn convert_message_tool_without_call_id_uses_unknown() {
        let msg = ChatMessage {
            role: proto::Role::Tool,
            content: "result".to_string(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: None,
        };
        let converted = convert_message(&msg).expect("tool without call_id");
        assert!(matches!(converted, ChatCompletionRequestMessage::Tool(_)));
    }
}
