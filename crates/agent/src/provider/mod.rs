//! LLM provider abstraction and shared types.

pub mod anthropic;
pub mod openai;
pub mod responses;

use async_trait::async_trait;
use proto::{LlmError, ToolCall, ToolDefinition};

/// Represents a message in a chat history
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// Semantic role of this message.
    pub role: proto::Role,
    /// Human-readable text content.
    pub content: String,
    /// Tool call id when this is a tool result.
    pub tool_call_id: Option<String>,
    /// Tool name when this is a tool result.
    pub tool_name: Option<String>,
    /// Tool calls requested by assistant messages.
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatMessage {
    /// Creates a system-role message with the given content.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: proto::Role::System,
            content: content.into(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: None,
        }
    }

    /// Creates a user-role message with the given content.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: proto::Role::User,
            content: content.into(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: None,
        }
    }

    /// Creates an assistant-role message with the given content.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: proto::Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: None,
        }
    }

    /// Creates a tool-result message linking a call id, tool name, and output content.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: proto::Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
            tool_calls: None,
        }
    }
}

/// Request to the LLM
#[derive(Debug)]
pub struct ChatRequest {
    /// Full chat history including system/user/assistant/tool messages.
    pub messages: Vec<ChatMessage>,
    /// Available tools schema.
    pub tools: Vec<ToolDefinition>,
    /// Target model id.
    pub model: String,
}

/// Token usage reported by the LLM for a single call.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Number of tokens in the prompt / input.
    pub prompt_tokens: u32,
    /// Number of tokens in the generated output.
    pub completion_tokens: u32,
}

impl TokenUsage {
    /// Accumulates another usage record into this one.
    pub fn add(&mut self, other: &TokenUsage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
    }
}

/// Response from the LLM
#[derive(Debug)]
pub enum ChatResponse {
    /// Final assistant text response.
    Text(String, TokenUsage),
    /// Assistant requested one or more tool calls.
    ToolCalls(Vec<ToolCall>, TokenUsage),
}

/// Sanitizes a tool name for provider APIs that restrict allowed characters.
/// Replaces any character that is not ASCII alphanumeric, `_`, or `-` with `_`.
pub(super) fn sanitize_tool_name(name: &str) -> String {
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

/// LLM provider trait
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Sends a chat request to the provider and returns either text or tool calls.
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_constructors_set_expected_roles() {
        let system = ChatMessage::system("s");
        assert_eq!(system.role, proto::Role::System);
        assert_eq!(system.content, "s");

        let user = ChatMessage::user("u");
        assert_eq!(user.role, proto::Role::User);

        let assistant = ChatMessage::assistant("a");
        assert_eq!(assistant.role, proto::Role::Assistant);

        let tool = ChatMessage::tool_result("call-1", "system.run", "ok");
        assert_eq!(tool.role, proto::Role::Tool);
        assert_eq!(tool.tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(tool.tool_name.as_deref(), Some("system.run"));
    }

    #[test]
    fn token_usage_default_is_zero() {
        let usage = TokenUsage::default();
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
    }

    #[test]
    fn token_usage_add_accumulates_values() {
        let mut total = TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
        };
        let other = TokenUsage {
            prompt_tokens: 5,
            completion_tokens: 15,
        };
        total.add(&other);
        assert_eq!(total.prompt_tokens, 15);
        assert_eq!(total.completion_tokens, 35);
    }

    #[test]
    fn token_usage_add_zero_is_identity() {
        let mut total = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 200,
        };
        total.add(&TokenUsage::default());
        assert_eq!(total.prompt_tokens, 100);
        assert_eq!(total.completion_tokens, 200);
    }

    #[test]
    fn chat_request_debug_format() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("hi")],
            tools: vec![],
            model: "gpt-4o".to_string(),
        };
        let debug = format!("{:?}", req);
        assert!(debug.contains("gpt-4o"));
    }

    #[test]
    fn chat_response_debug_format() {
        let resp = ChatResponse::Text("hello".to_string(), TokenUsage::default());
        let debug = format!("{:?}", resp);
        assert!(debug.contains("hello"));
    }

    #[test]
    fn chat_message_debug_and_clone() {
        let msg = ChatMessage::user("test");
        let cloned = msg.clone();
        assert_eq!(cloned.content, "test");
        let debug = format!("{:?}", msg);
        assert!(debug.contains("User"));
    }
}
