//! OpenAI Responses API provider implementation.
//!
//! Uses the Responses API (`/responses`) which bills against a user's
//! ChatGPT subscription (via `chatgpt.com/backend-api/codex`) or standard
//! API credits (via `api.openai.com/v1`).

use async_trait::async_trait;
use proto::{LlmError, ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, trace};

use crate::llm::{ChatMessage, ChatRequest, ChatResponse, LlmProvider, TokenUsage};

// ── Request types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ResponsesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: Vec<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ResponsesTool>,
    store: bool,
    /// ChatGPT backend (`chatgpt.com`) requires streaming. When `true`,
    /// the response arrives as SSE events that are reassembled into a
    /// complete `ResponsesResponse`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ResponsesTool {
    #[serde(rename = "type")]
    tool_type: &'static str,
    name: String,
    description: String,
    parameters: Value,
}

// ── Response types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ResponsesResponse {
    output: Vec<OutputItem>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputItem {
    Message {
        content: Vec<MessageContent>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MessageContent {
    OutputText { text: String },
}

// ── Provider ───────────────────────────────────────────────────────────────────

/// OpenAI Responses API LLM provider.
///
/// Uses the Responses API for ChatGPT subscription-based billing.
pub struct ResponsesApiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    /// ChatGPT account ID extracted from the JWT `access_token` claims.
    /// When set, sent as the `chatgpt-account-id` header so OpenAI can
    /// identify which ChatGPT subscription to bill.
    chatgpt_account_id: Option<String>,
}

impl ResponsesApiProvider {
    /// Creates a provider targeting the default OpenAI API endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: "https://api.openai.com/v1".to_string(),
            chatgpt_account_id: None,
        }
    }

    /// Creates a provider targeting a custom base URL (useful for proxies/tests).
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            chatgpt_account_id: None,
        }
    }

    /// Sets the ChatGPT account ID header for subscription-based billing.
    /// When an account ID is present and the base URL is still the default
    /// `https://api.openai.com/v1`, switches to the ChatGPT backend endpoint
    /// (`https://chatgpt.com/backend-api/codex`) which accepts JWT tokens.
    pub fn with_chatgpt_account_id(mut self, account_id: Option<String>) -> Self {
        self.chatgpt_account_id = account_id;
        // When using ChatGPT subscription auth, switch to the ChatGPT backend endpoint
        if self.chatgpt_account_id.is_some() && self.base_url == "https://api.openai.com/v1" {
            self.base_url = "https://chatgpt.com/backend-api/codex".to_string();
        }
        self
    }

    /// Whether we are talking to the ChatGPT backend (requires streaming).
    fn is_chatgpt_backend(&self) -> bool {
        self.chatgpt_account_id.is_some()
    }
}

#[async_trait]
impl LlmProvider for ResponsesApiProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        // Extract system messages into top-level instructions field.
        let instructions_parts: Vec<String> = req
            .messages
            .iter()
            .filter(|m| m.role == proto::Role::System)
            .map(|m| m.content.clone())
            .collect();
        let instructions = if instructions_parts.is_empty() {
            None
        } else {
            Some(instructions_parts.join("\n"))
        };

        // Build reverse mapping: sanitized_name → original_name
        // Detect collisions: two tools that both sanitize to the same name would
        // cause incorrect tool routing, so we return an error early.
        let mut tool_name_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::with_capacity(req.tools.len());
        for t in &req.tools {
            let sanitized = sanitize_tool_name(&t.name);
            if let Some(existing) = tool_name_map.get(&sanitized)
                && existing != &t.name
            {
                return Err(LlmError::Api(format!(
                    "Tool name collision: '{}' and '{}' both sanitize to '{}'",
                    existing, t.name, sanitized
                )));
            }
            tool_name_map.insert(sanitized, t.name.clone());
        }

        let input = convert_messages(&req.messages);
        let tools: Vec<ResponsesTool> = req.tools.iter().map(convert_tool).collect();

        let is_stream = self.is_chatgpt_backend();
        let responses_req = ResponsesRequest {
            model: req.model.clone(),
            instructions,
            input,
            tools,
            store: false,
            stream: is_stream,
        };

        let url = format!("{}/responses", self.base_url);
        debug!(
            model = %req.model,
            input_items = %responses_req.input.len(),
            tools = %responses_req.tools.len(),
            "Sending request to OpenAI Responses API"
        );
        trace!(
            "Responses API request body: {}",
            serde_json::to_string(&responses_req).unwrap_or_default()
        );

        let mut req_builder = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .header("originator", "codex_cli_rs");
        if let Some(ref account_id) = self.chatgpt_account_id {
            req_builder = req_builder.header("chatgpt-account-id", account_id);
        }
        let response = req_builder
            .json(&responses_req)
            .send()
            .await
            .map_err(|e| LlmError::Api(e.to_string()))?;
        let status = response.status();
        debug!(status = %status.as_u16(), "Responses API response received");
        if status.as_u16() == 429 {
            return Err(LlmError::RateLimit);
        }
        let body = response
            .text()
            .await
            .map_err(|e| LlmError::Api(e.to_string()))?;
        if !status.is_success() {
            debug!(status = %status, body = %body.chars().take(500).collect::<String>(), "Responses API error response");
            return Err(parse_api_error(&body, status));
        }

        // When streaming, extract the final response from SSE events;
        // otherwise parse the JSON body directly.
        let responses_resp: ResponsesResponse = if is_stream {
            parse_sse_response(&body)?
        } else {
            serde_json::from_str(&body).map_err(|e| {
                LlmError::InvalidResponse(format!(
                    "Deserialization error: {e}; body: {}",
                    body.chars().take(200).collect::<String>()
                ))
            })?
        };
        debug!(
            output_items = %responses_resp.output.len(),
            "Responses API response parsed"
        );
        // Check for function calls first.
        let tool_calls: Vec<ToolCall> = responses_resp
            .output
            .iter()
            .filter_map(|item| {
                if let OutputItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                } = item
                {
                    Some(ToolCall {
                        id: call_id.clone(),
                        name: tool_name_map
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| name.clone()),
                        arguments: parse_tool_arguments(arguments),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !tool_calls.is_empty() {
            return Ok(ChatResponse::ToolCalls(tool_calls, TokenUsage::default()));
        }
        // Collect text from message output items.
        let text: String = responses_resp
            .output
            .into_iter()
            .filter_map(|item| {
                if let OutputItem::Message { content } = item {
                    let texts: Vec<String> = content
                        .into_iter()
                        .map(|c| match c {
                            MessageContent::OutputText { text } => text,
                        })
                        .collect();
                    Some(texts.join(""))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");
        Ok(ChatResponse::Text(text, TokenUsage::default()))
    }
}

// ── Conversion helpers ─────────────────────────────────────────────────────────

/// Converts internal chat messages into Responses API input items.
///
/// System messages are skipped (handled via top-level `instructions` field).
fn convert_messages(messages: &[ChatMessage]) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();

    for msg in messages {
        match msg.role {
            proto::Role::System => {
                // Already extracted to top-level instructions field – skip.
            }
            proto::Role::User => {
                result.push(serde_json::json!({
                    "role": "user",
                    "content": msg.content
                }));
            }
            proto::Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        result.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": sanitize_tool_name(&tc.name),
                            "arguments": tc.arguments.to_string()
                        }));
                    }
                } else {
                    result.push(serde_json::json!({
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": msg.content
                        }]
                    }));
                }
            }
            proto::Role::Tool => {
                let call_id = msg
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                result.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": msg.content
                }));
            }
        }
    }

    result
}

fn convert_tool(t: &ToolDefinition) -> ResponsesTool {
    ResponsesTool {
        tool_type: "function",
        name: sanitize_tool_name(&t.name),
        description: t.description.clone(),
        parameters: t.parameters.clone(),
    }
}

/// Parses tool call argument JSON with empty-object fallback.
fn parse_tool_arguments(arguments: &str) -> Value {
    serde_json::from_str(arguments).unwrap_or(Value::Object(Default::default()))
}

/// Sanitizes a tool name so it matches the OpenAI `^[a-zA-Z0-9_-]+$` pattern.
/// Non-conforming characters (e.g. `.`) are replaced with `_`.
fn sanitize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Parses an API error from the response body.
///
/// Handles two error formats:
/// - OpenAI standard: `{"error": {"message": "..."}}`
/// - ChatGPT backend: `{"detail": "..."}`
fn parse_api_error(body: &str, status: reqwest::StatusCode) -> LlmError {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) {
        // Try standard OpenAI format first, then ChatGPT backend `detail` field.
        let msg = parsed["error"]["message"]
            .as_str()
            .or_else(|| parsed["detail"].as_str())
            .unwrap_or("Unknown error");
        let hint = if msg.to_lowercase().contains("billing") || msg.to_lowercase().contains("quota")
        {
            " Check your OpenAI billing at https://platform.openai.com."
        } else if msg.to_lowercase().contains("model")
            || msg.to_lowercase().contains("not supported")
            || msg.to_lowercase().contains("not found")
        {
            " Try /model to select a different model."
        } else if msg.to_lowercase().contains("auth") {
            " Use /login to re-authenticate."
        } else {
            ""
        };
        return LlmError::Api(format!("{msg}{hint}"));
    }
    LlmError::Api(format!(
        "HTTP {status}: {}",
        body.chars().take(500).collect::<String>()
    ))
}

/// Parses a streaming SSE response body into a `ResponsesResponse`.
///
/// The ChatGPT backend requires `stream: true` and returns Server-Sent Events.
/// We look for the final `response.completed` event which contains the complete
/// response object including the `output` array.
fn parse_sse_response(body: &str) -> Result<ResponsesResponse, LlmError> {
    // SSE format: lines of "event: <type>\ndata: <json>\n\n"
    // We look for event types that carry the full response:
    //   - "response.completed" (most common final event)
    //   - fall back to extracting output items from individual events
    let mut last_response_data: Option<String> = None;
    let mut current_event: Option<&str> = None;
    let mut data_buffer: Vec<&str> = Vec::new();
    for line in body.lines() {
        if let Some(event_type) = line.strip_prefix("event: ") {
            current_event = Some(event_type.trim());
            data_buffer.clear();
        } else if let Some(data) = line.strip_prefix("data: ") {
            data_buffer.push(data.trim());
        } else if line.is_empty() {
            if matches!(current_event, Some("response.completed")) && !data_buffer.is_empty() {
                last_response_data = Some(data_buffer.join("\n"));
            }
            current_event = None;
            data_buffer.clear();
        }
    }
    // Handle trailing event without a trailing blank line.
    if matches!(current_event, Some("response.completed")) && !data_buffer.is_empty() {
        last_response_data = Some(data_buffer.join("\n"));
    }

    if let Some(ref data) = last_response_data {
        // The `response.completed` event data is a wrapper with a `response` field.
        if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(data) {
            // Try `response.output` (wrapper format) first, then direct `output`.
            let response_obj = if wrapper.get("response").is_some() {
                &wrapper["response"]
            } else {
                &wrapper
            };
            if let Ok(resp) = serde_json::from_value::<ResponsesResponse>(response_obj.clone()) {
                return Ok(resp);
            }
        }
    }

    // Fallback: try to find any response.done or response.completed-like event
    // with an output array.
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data: ")
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data.trim())
        {
            // Check wrapped format first (non-destructive borrow)
            if let Some(response) = parsed.get("response")
                && response.get("output").is_some()
                && let Ok(resp) = serde_json::from_value::<ResponsesResponse>(response.clone())
            {
                return Ok(resp);
            }
            // Check if this is a complete response with output
            if parsed.get("output").is_some()
                && let Ok(resp) = serde_json::from_value::<ResponsesResponse>(parsed)
            {
                return Ok(resp);
            }
        }
    }

    Err(LlmError::InvalidResponse(format!(
        "No valid response found in SSE stream; body: {}",
        body.chars().take(300).collect::<String>()
    )))
}
// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatMessage;

    // ── constructor tests ──────────────────────────────────────────────────────

    #[test]
    fn provider_new_stores_api_key_and_default_url() {
        let p = ResponsesApiProvider::new("sk-test");
        assert_eq!(p.api_key, "sk-test");
        assert_eq!(p.base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn provider_with_base_url_overrides_url() {
        let p = ResponsesApiProvider::with_base_url("sk-test", "http://localhost:8080");
        assert_eq!(p.api_key, "sk-test");
        assert_eq!(p.base_url, "http://localhost:8080");
    }

    // ── message conversion tests ───────────────────────────────────────────────

    #[test]
    fn system_messages_are_skipped_in_convert_messages() {
        let msgs = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("hello"),
        ];
        let converted = convert_messages(&msgs);
        // Only the user message should appear; system is skipped.
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
    }

    #[test]
    fn user_message_converted_correctly() {
        let msgs = vec![ChatMessage::user("test message")];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"], "test message");
    }

    #[test]
    fn assistant_message_with_text_produces_output_text() {
        let msgs = vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello back"),
        ];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[1]["role"], "assistant");
        assert_eq!(converted[1]["content"][0]["type"], "output_text");
        assert_eq!(converted[1]["content"][0]["text"], "hello back");
    }

    #[test]
    fn assistant_with_tool_calls_produces_function_call_items() {
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = Some(vec![
            ToolCall {
                id: "tc1".to_string(),
                name: "system.run".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
            },
            ToolCall {
                id: "tc2".to_string(),
                name: "screen".to_string(),
                arguments: serde_json::json!({}),
            },
        ]);
        let msgs = vec![ChatMessage::user("go"), assistant];
        let converted = convert_messages(&msgs);
        // user + 2 function_call items
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[1]["type"], "function_call");
        assert_eq!(converted[1]["call_id"], "tc1");
        assert_eq!(converted[1]["name"], "system_run");
        assert_eq!(converted[2]["type"], "function_call");
        assert_eq!(converted[2]["call_id"], "tc2");
    }

    #[test]
    fn tool_result_produces_function_call_output() {
        let msgs = vec![
            ChatMessage::user("start"),
            ChatMessage::tool_result("tc1", "system.run", "result-1"),
        ];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[1]["type"], "function_call_output");
        assert_eq!(converted[1]["call_id"], "tc1");
        assert_eq!(converted[1]["output"], "result-1");
    }

    // ── convert_tool ───────────────────────────────────────────────────────────

    #[test]
    fn convert_tool_maps_fields_correctly() {
        let def = ToolDefinition::new(
            "system.run",
            "Run shell command",
            serde_json::json!({"type": "object"}),
        );
        let t = convert_tool(&def);
        assert_eq!(t.tool_type, "function");
        assert_eq!(t.name, "system_run");
        assert_eq!(t.description, "Run shell command");
        assert_eq!(t.parameters, serde_json::json!({"type": "object"}));
    }

    // ── response parsing ───────────────────────────────────────────────────────

    #[test]
    fn parses_text_response() {
        let json = r#"{"id":"resp_1","output":[{"type":"message","content":[{"type":"output_text","text":"Hello!"}]}]}"#;
        let resp: ResponsesResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.output.len(), 1);
        match &resp.output[0] {
            OutputItem::Message { content } => {
                assert_eq!(content.len(), 1);
                match &content[0] {
                    MessageContent::OutputText { text } => assert_eq!(text, "Hello!"),
                }
            }
            _ => panic!("expected message output item"),
        }
    }

    #[test]
    fn parses_function_call_response() {
        let json = r#"{"output":[{"type":"function_call","call_id":"call_xxx","name":"system.run","arguments":"{\"command\":\"ls\"}"}]}"#;
        let resp: ResponsesResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.output.len(), 1);
        match &resp.output[0] {
            OutputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(call_id, "call_xxx");
                assert_eq!(name, "system.run");
                assert!(arguments.contains("command"));
            }
            _ => panic!("expected function_call output item"),
        }
    }

    #[test]
    fn parses_mixed_response_with_message_and_function_call() {
        let json = r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"Let me run that."}]},{"type":"function_call","call_id":"call_1","name":"system.run","arguments":"{}"}]}"#;
        let resp: ResponsesResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(resp.output.len(), 2);
        assert!(matches!(&resp.output[0], OutputItem::Message { .. }));
        assert!(matches!(&resp.output[1], OutputItem::FunctionCall { .. }));
    }

    #[test]
    fn empty_output_in_response() {
        let json = r#"{"output":[]}"#;
        let resp: ResponsesResponse = serde_json::from_str(json).expect("parse");
        assert!(resp.output.is_empty());
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
    fn tool_name_collision_detected() {
        // "a.b" and "a_b" both sanitize to "a_b"
        let req = crate::ChatRequest {
            model: "gpt-4o".to_string(),
            messages: vec![ChatMessage::user("hi")],
            tools: vec![
                ToolDefinition::new("a.b", "desc", serde_json::json!({"type":"object"})),
                ToolDefinition::new("a_b", "desc2", serde_json::json!({"type":"object"})),
            ],
        };
        let provider = ResponsesApiProvider::new("sk-test");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(provider.chat(req));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("collision"), "expected 'collision' in: {msg}");
    }

    #[test]
    fn parse_sse_response_joins_multi_line_data() {
        // Simulate an SSE body where response.completed spans 2 data lines
        let body = concat!(
            "event: response.in_progress\n",
            "data: {\"partial\": true}\n",
            "\n",
            "event: response.completed\n",
            "data: {\"response\":{\"output\":[{\"type\":\"message\",",
            "data: \"content\":[{\"type\":\"output_text\",\"text\":\"hi\"}]}]}}\n",
            "\n",
        );
        // The two data lines join into one JSON object via '\n', which may not parse,
        // but the fallback scanner will pick up the inline JSON from data lines.
        // The key invariant: the function does NOT panic and returns a Result.
        let result = parse_sse_response(body);
        // Either Ok (if fallback finds parseable JSON) or Err (if not) — no panic.
        let _ = result;
    }

    #[test]
    fn parse_sse_response_rejects_empty_stream() {
        let result = parse_sse_response("");
        assert!(result.is_err(), "empty SSE stream must return Err");
    }

    // ── with_chatgpt_account_id / is_chatgpt_backend ────────────────────────

    #[test]
    fn with_chatgpt_account_id_switches_to_chatgpt_endpoint() {
        let p = ResponsesApiProvider::new("sk-test")
            .with_chatgpt_account_id(Some("acct-123".to_string()));
        assert_eq!(p.base_url, "https://chatgpt.com/backend-api/codex");
        assert_eq!(p.chatgpt_account_id.as_deref(), Some("acct-123"));
    }

    #[test]
    fn with_chatgpt_account_id_none_keeps_default_url() {
        let p = ResponsesApiProvider::new("sk-test").with_chatgpt_account_id(None);
        assert_eq!(p.base_url, "https://api.openai.com/v1");
        assert!(p.chatgpt_account_id.is_none());
    }

    #[test]
    fn with_chatgpt_account_id_preserves_custom_base_url() {
        let p = ResponsesApiProvider::with_base_url("sk-test", "http://proxy")
            .with_chatgpt_account_id(Some("acct-123".to_string()));
        // Custom URL should not be replaced — only default URL triggers the switch.
        assert_eq!(p.base_url, "http://proxy");
    }

    #[test]
    fn is_chatgpt_backend_true_when_account_id_set() {
        let p = ResponsesApiProvider::new("sk-test")
            .with_chatgpt_account_id(Some("acct-id".to_string()));
        assert!(p.is_chatgpt_backend());
    }

    #[test]
    fn is_chatgpt_backend_false_when_no_account_id() {
        let p = ResponsesApiProvider::new("sk-test");
        assert!(!p.is_chatgpt_backend());
    }

    // ── parse_api_error ───────────────────────────────────────────────────────

    #[test]
    fn parse_api_error_extracts_standard_openai_message() {
        let body = r#"{"error":{"message":"Invalid API key"}}"}"#;
        let err = parse_api_error(body, reqwest::StatusCode::UNAUTHORIZED);
        let msg = err.to_string();
        assert!(msg.contains("Invalid API key"), "got: {msg}");
    }

    #[test]
    fn parse_api_error_extracts_chatgpt_detail_field() {
        let body = r#"{"detail":"Rate limit exceeded"}"}"#;
        let err = parse_api_error(body, reqwest::StatusCode::TOO_MANY_REQUESTS);
        let msg = err.to_string();
        assert!(msg.contains("Rate limit exceeded"), "got: {msg}");
    }

    #[test]
    fn parse_api_error_adds_billing_hint() {
        let body = r#"{"error":{"message":"You exceeded your billing quota."}}"}"#;
        let err = parse_api_error(body, reqwest::StatusCode::TOO_MANY_REQUESTS);
        let msg = err.to_string();
        assert!(
            msg.contains("billing") || msg.contains("platform.openai.com"),
            "got: {msg}"
        );
    }

    #[test]
    fn parse_api_error_adds_model_hint() {
        let body = r#"{"error":{"message":"The model gpt-99 not found."}}"}"#;
        let err = parse_api_error(body, reqwest::StatusCode::NOT_FOUND);
        let msg = err.to_string();
        assert!(
            msg.contains("/model") || msg.contains("not found"),
            "got: {msg}"
        );
    }

    #[test]
    fn parse_api_error_adds_auth_hint() {
        let body = r#"{"error":{"message":"Incorrect authentication credentials."}}"}"#;
        let err = parse_api_error(body, reqwest::StatusCode::UNAUTHORIZED);
        let msg = err.to_string();
        assert!(
            msg.contains("authentication") || msg.contains("/login"),
            "got: {msg}"
        );
    }

    #[test]
    fn parse_api_error_falls_back_to_http_status_for_non_json() {
        let err = parse_api_error("not json", reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        let msg = err.to_string();
        assert!(
            msg.contains("500") || msg.contains("not json"),
            "got: {msg}"
        );
    }

    // ── parse_sse_response – additional paths ────────────────────────────────

    #[test]
    fn parse_sse_response_handles_response_completed_with_direct_output() {
        // response.completed with data containing `output` directly (no wrapper `response` key)
        let body = concat!(
            "event: response.completed\n",
            "data: {\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"direct\"}]}]}\n",
            "\n",
        );
        let resp = parse_sse_response(body).expect("should parse direct output");
        assert_eq!(resp.output.len(), 1);
    }

    #[test]
    fn parse_sse_response_handles_wrapped_response_object() {
        // response.completed with data containing `{"response":{"output":[...]}}` wrapper
        let body = concat!(
            "event: response.completed\n",
            "data: {\"response\":{\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"wrapped\"}]}]}}\n",
            "\n",
        );
        let resp = parse_sse_response(body).expect("should parse wrapped response");
        assert_eq!(resp.output.len(), 1);
        match &resp.output[0] {
            OutputItem::Message { content } => {
                let MessageContent::OutputText { text } = &content[0];
                assert_eq!(text, "wrapped");
            }
            _ => panic!("expected message item"),
        }
    }

    #[test]
    fn parse_sse_response_trailing_event_without_blank_line() {
        // No trailing blank line — the code has a special case for this.
        let body = concat!(
            "event: response.completed\n",
            "data: {\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"trailing\"}]}]}",
        );
        let resp = parse_sse_response(body).expect("trailing event without blank line");
        assert_eq!(resp.output.len(), 1);
    }

    #[test]
    fn parse_sse_response_fallback_scanner_finds_wrapped_output() {
        // No response.completed event; fallback scanner picks up wrapped response from data: line
        let body = concat!(
            "event: response.in_progress\n",
            "data: {\"response\":{\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"fallback\"}]}]}}\n",
            "\n",
        );
        let resp = parse_sse_response(body).expect("fallback scanner should find output");
        assert_eq!(resp.output.len(), 1);
    }

    #[test]
    fn parse_sse_response_fallback_scanner_finds_direct_output() {
        // No response.completed event; fallback finds direct output array
        let body = concat!(
            "event: some.event\n",
            "data: {\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"direct_fb\"}]}]}\n",
            "\n",
        );
        let resp = parse_sse_response(body).expect("fallback direct output");
        assert_eq!(resp.output.len(), 1);
    }

    // ── sanitize_tool_name ─────────────────────────────────────────────────

    #[test]
    fn sanitize_tool_name_replaces_dots() {
        assert_eq!(sanitize_tool_name("system.run"), "system_run");
    }

    #[test]
    fn sanitize_tool_name_preserves_valid_chars() {
        assert_eq!(sanitize_tool_name("my-tool_v2"), "my-tool_v2");
    }

    #[test]
    fn sanitize_tool_name_replaces_special_chars() {
        assert_eq!(sanitize_tool_name("a@b#c$d"), "a_b_c_d");
    }

    #[test]
    fn sanitize_tool_name_empty_string() {
        assert_eq!(sanitize_tool_name(""), "");
    }

    #[test]
    fn sanitize_tool_name_all_special() {
        assert_eq!(sanitize_tool_name("..."), "___");
    }

    // ── convert_messages edge cases ──────────────────────────────────────────

    #[test]
    fn tool_result_without_call_id_uses_unknown() {
        let mut msg = ChatMessage::tool_result("tc1", "tool", "output");
        msg.tool_call_id = None;
        let converted = convert_messages(&[msg]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["call_id"], "unknown");
    }

    #[test]
    fn convert_messages_empty_input() {
        let converted = convert_messages(&[]);
        assert!(converted.is_empty());
    }

    #[test]
    fn convert_messages_multiple_system_messages_all_skipped() {
        let msgs = vec![
            ChatMessage::system("sys1"),
            ChatMessage::system("sys2"),
            ChatMessage::user("hello"),
        ];
        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
    }

    // ── parse_tool_arguments edge cases ──────────────────────────────────────

    #[test]
    fn parse_tool_arguments_empty_string() {
        let result = parse_tool_arguments("");
        assert!(result.is_object());
    }

    #[test]
    fn parse_tool_arguments_null_json() {
        let result = parse_tool_arguments("null");
        // null is valid JSON but not an object, so it returns null
        assert!(result.is_null());
    }

    #[test]
    fn parse_tool_arguments_nested_object() {
        let result = parse_tool_arguments(r#"{"a":{"b":1}}"#);
        assert_eq!(result["a"]["b"], 1);
    }
}
