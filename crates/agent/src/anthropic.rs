//! Anthropic Messages API provider implementation.

use async_trait::async_trait;
use proto::{LlmError, ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::llm::{ChatMessage, ChatRequest, ChatResponse, LlmProvider};

const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 8192;

// ── Request types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: AnthropicContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
}

// ── Response types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
    stop_reason: Option<String>,
}

// ── Provider ───────────────────────────────────────────────────────────────────

/// Anthropic Messages API LLM provider.
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    /// Creates a provider targeting the default Anthropic API endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Creates a provider targeting a custom base URL (useful for proxies/tests).
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        // Extract system messages into top-level system field (Anthropic requirement).
        let system_parts: Vec<String> = req
            .messages
            .iter()
            .filter(|m| m.role == proto::Role::System)
            .map(|m| m.content.clone())
            .collect();
        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n"))
        };

        let messages = convert_messages(&req.messages)?;
        let tools: Vec<AnthropicTool> = req.tools.iter().map(convert_tool).collect();

        let anthropic_req = AnthropicRequest {
            model: req.model.clone(),
            max_tokens: MAX_TOKENS,
            system,
            messages,
            tools,
        };

        let url = format!("{}/v1/messages", self.base_url);
        debug!(
            model = %req.model,
            messages = %anthropic_req.messages.len(),
            tools = %anthropic_req.tools.len(),
            "Sending request to Anthropic"
        );

        let mut req_builder = self
            .client
            .post(&url)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json");

        if proto::is_anthropic_oauth_token(&self.api_key) {
            req_builder = req_builder.bearer_auth(&self.api_key);
        } else {
            req_builder = req_builder.header("x-api-key", &self.api_key);
        }

        let response = req_builder
            .json(&anthropic_req)
            .send()
            .await
            .map_err(|e| LlmError::Api(e.to_string()))?;

        let status = response.status();
        debug!(status = %status.as_u16(), "Anthropic response received");
        if status.as_u16() == 429 {
            return Err(LlmError::RateLimit);
        }

        let body = response
            .text()
            .await
            .map_err(|e| LlmError::Api(e.to_string()))?;

        if !status.is_success() {
            let preview: String = body.chars().take(500).collect();
            return Err(LlmError::Api(format!("HTTP {status}: {preview}")));
        }

        let anthropic_resp: AnthropicResponse = serde_json::from_str(&body).map_err(|e| {
            LlmError::InvalidResponse(format!(
                "Deserialization error: {e}; body: {}",
                body.chars().take(200).collect::<String>()
            ))
        })?;

        if anthropic_resp.stop_reason.as_deref() == Some("tool_use") {
            let tool_calls: Vec<ToolCall> = anthropic_resp
                .content
                .into_iter()
                .filter_map(|block| {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        Some(ToolCall {
                            id,
                            name,
                            arguments: input,
                        })
                    } else {
                        None
                    }
                })
                .collect();
            return Ok(ChatResponse::ToolCalls(tool_calls));
        }

        let text = anthropic_resp
            .content
            .into_iter()
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    Some(text)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(ChatResponse::Text(text))
    }
}

// ── Conversion helpers ─────────────────────────────────────────────────────────

/// Converts internal chat messages into Anthropic format.
///
/// System messages are skipped (handled via top-level `system` field).
/// Consecutive `Role::Tool` messages are merged into a single user message
/// with multiple `tool_result` blocks (Anthropic forbids consecutive same-role
/// messages).
fn convert_messages(messages: &[ChatMessage]) -> Result<Vec<AnthropicMessage>, LlmError> {
    let mut result: Vec<AnthropicMessage> = Vec::new();

    for msg in messages {
        match msg.role {
            proto::Role::System => {
                // Already extracted to top-level system field – skip.
            }
            proto::Role::User => {
                result.push(AnthropicMessage {
                    role: "user",
                    content: AnthropicContent::Text(msg.content.clone()),
                });
            }
            proto::Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    let blocks: Vec<ContentBlock> = tool_calls
                        .iter()
                        .map(|tc| ContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            input: tc.arguments.clone(),
                        })
                        .collect();
                    result.push(AnthropicMessage {
                        role: "assistant",
                        content: AnthropicContent::Blocks(blocks),
                    });
                } else {
                    result.push(AnthropicMessage {
                        role: "assistant",
                        content: AnthropicContent::Text(msg.content.clone()),
                    });
                }
            }
            proto::Role::Tool => {
                let tool_use_id = msg
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());

                // Merge into last user message if it already holds tool_result blocks.
                let merged = if let Some(last) = result.last_mut() {
                    if last.role == "user" {
                        if let AnthropicContent::Blocks(ref mut blocks) = last.content {
                            blocks.push(ContentBlock::ToolResult {
                                tool_use_id: tool_use_id.clone(),
                                content: msg.content.clone(),
                            });
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !merged {
                    result.push(AnthropicMessage {
                        role: "user",
                        content: AnthropicContent::Blocks(vec![ContentBlock::ToolResult {
                            tool_use_id,
                            content: msg.content.clone(),
                        }]),
                    });
                }
            }
        }
    }

    Ok(result)
}

fn convert_tool(t: &ToolDefinition) -> AnthropicTool {
    AnthropicTool {
        name: t.name.clone(),
        description: t.description.clone(),
        input_schema: t.parameters.clone(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatMessage;

    // ── constructor tests ──────────────────────────────────────────────────────

    #[test]
    fn provider_new_stores_api_key_and_default_url() {
        let p = AnthropicProvider::new("sk-test");
        assert_eq!(p.api_key, "sk-test");
        assert_eq!(p.base_url, "https://api.anthropic.com");
    }

    #[test]
    fn provider_with_base_url_overrides_url() {
        let p = AnthropicProvider::with_base_url("sk-test", "http://localhost:8080");
        assert_eq!(p.base_url, "http://localhost:8080");
    }

    // ── message conversion tests ───────────────────────────────────────────────

    #[test]
    fn system_messages_are_skipped_in_convert_messages() {
        let msgs = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("hello"),
        ];
        let converted = convert_messages(&msgs).expect("conversion");
        // Only the user message should appear; system is skipped.
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
    }

    #[test]
    fn assistant_with_tool_calls_produces_tool_use_blocks() {
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = Some(vec![
            ToolCall {
                id: "tc1".to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
            },
            ToolCall {
                id: "tc2".to_string(),
                name: "screen".to_string(),
                arguments: serde_json::json!({}),
            },
        ]);
        let msgs = vec![ChatMessage::user("go"), assistant];
        let converted = convert_messages(&msgs).expect("conversion");
        assert_eq!(converted.len(), 2);

        let AnthropicContent::Blocks(ref blocks) = converted[1].content else {
            panic!("expected blocks for assistant with tool calls");
        };
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], ContentBlock::ToolUse { id, .. } if id == "tc1"));
        assert!(matches!(&blocks[1], ContentBlock::ToolUse { id, .. } if id == "tc2"));
    }

    #[test]
    fn consecutive_tool_results_merged_into_single_user_message() {
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = Some(vec![
            ToolCall {
                id: "tc1".to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({}),
            },
            ToolCall {
                id: "tc2".to_string(),
                name: "screen".to_string(),
                arguments: serde_json::json!({}),
            },
        ]);
        let msgs = vec![
            ChatMessage::user("start"),
            assistant,
            ChatMessage::tool_result("tc1", "bash", "result-1"),
            ChatMessage::tool_result("tc2", "screen", "result-2"),
        ];

        let converted = convert_messages(&msgs).expect("conversion");
        // Expected: [user, assistant, user(merged tool results)]
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[2].role, "user");

        let AnthropicContent::Blocks(ref blocks) = converted[2].content else {
            panic!("expected blocks in merged user message");
        };
        assert_eq!(blocks.len(), 2);
        assert!(
            matches!(&blocks[0], ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "tc1")
        );
        assert!(
            matches!(&blocks[1], ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "tc2")
        );
    }

    #[test]
    fn tool_result_not_merged_when_preceded_by_assistant_message() {
        // If there's an assistant message right before the tool result (not a
        // user-with-blocks), a new user message should be created.
        let msgs = vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant("I'll help"),
            ChatMessage::tool_result("tc1", "bash", "output"),
        ];
        let converted = convert_messages(&msgs).expect("conversion");
        // [user, assistant, user(tool result)] — 3 entries
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[2].role, "user");
    }

    // ── convert_tool ───────────────────────────────────────────────────────────

    #[test]
    fn convert_tool_maps_fields_correctly() {
        let def = ToolDefinition::new(
            "bash",
            "Run shell command",
            serde_json::json!({"type": "object"}),
        );
        let t = convert_tool(&def);
        assert_eq!(t.name, "bash");
        assert_eq!(t.description, "Run shell command");
        assert_eq!(t.input_schema, serde_json::json!({"type": "object"}));
    }

    // ── response parsing via serde ─────────────────────────────────────────────

    #[test]
    fn parses_text_response() {
        let json = r#"{"content":[{"type":"text","text":"Hello!"}],"stop_reason":"end_turn"}"#;
        let resp: AnthropicResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
        assert!(matches!(&resp.content[0], ContentBlock::Text { text } if text == "Hello!"));
    }

    #[test]
    fn parses_tool_use_response() {
        let json = r#"{
            "content": [{"type":"tool_use","id":"tu1","name":"bash","input":{"command":"ls"}}],
            "stop_reason": "tool_use"
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.stop_reason.as_deref(), Some("tool_use"));
        let ContentBlock::ToolUse { id, name, input } = &resp.content[0] else {
            panic!("expected tool_use block");
        };
        assert_eq!(id, "tu1");
        assert_eq!(name, "bash");
        assert_eq!(input["command"], "ls");
    }

    #[test]
    fn serializes_tool_result_content_block() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu1".to_string(),
            content: "ok".to_string(),
        };
        let json = serde_json::to_string(&block).expect("serialize");
        assert!(json.contains("\"type\":\"tool_result\""));
        assert!(json.contains("\"tool_use_id\":\"tu1\""));
    }

    #[test]
    fn parses_mixed_text_and_tool_use_response() {
        let json = r#"{
            "content": [
                {"type":"text","text":"Let me run that."},
                {"type":"tool_use","id":"tu1","name":"bash","input":{"command":"ls"}}
            ],
            "stop_reason": "tool_use"
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.content.len(), 2);
        assert!(
            matches!(&resp.content[0], ContentBlock::Text { text } if text == "Let me run that.")
        );
        assert!(matches!(&resp.content[1], ContentBlock::ToolUse { name, .. } if name == "bash"));
    }

    #[test]
    fn empty_content_in_response() {
        let json = r#"{"content":[],"stop_reason":"end_turn"}"#;
        let resp: AnthropicResponse = serde_json::from_str(json).expect("parse");
        assert!(resp.content.is_empty());
    }

    #[test]
    fn system_message_extracted_properly() {
        let msgs = vec![
            ChatMessage::system("Be helpful"),
            ChatMessage::user("hello"),
        ];
        let converted = convert_messages(&msgs).expect("conversion");
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
    }

    #[test]
    fn user_message_with_plain_text_content() {
        let msgs = vec![ChatMessage::user("test message")];
        let converted = convert_messages(&msgs).expect("conversion");
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
        let AnthropicContent::Text(ref text) = converted[0].content else {
            panic!("expected text content");
        };
        assert_eq!(text, "test message");
    }

    #[test]
    fn assistant_message_with_plain_text_content() {
        let msgs = vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello back"),
        ];
        let converted = convert_messages(&msgs).expect("conversion");
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[1].role, "assistant");
        let AnthropicContent::Text(ref text) = converted[1].content else {
            panic!("expected text content");
        };
        assert_eq!(text, "hello back");
    }

    #[test]
    fn oauth_token_detected_by_shared_helper() {
        // Verify the shared helper correctly identifies OAuth tokens
        assert!(proto::is_anthropic_oauth_token("sk-ant-oat01-abc123"));
        assert!(!proto::is_anthropic_oauth_token("sk-ant-api03-abc123"));
        assert!(!proto::is_anthropic_oauth_token(""));
    }
}
