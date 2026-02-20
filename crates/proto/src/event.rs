use serde::{Deserialize, Serialize};

use crate::message::{ChannelId, SessionId};

/// Inbound event from a channel adapter to the agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEvent {
    /// Source channel identifier.
    pub channel_id: ChannelId,
    /// Session identifier associated with this inbound event.
    pub session_id: SessionId,
    /// User text payload.
    pub user_message: String,
    /// Optional structured metadata attached by adapter/runtime.
    pub metadata: Option<serde_json::Value>,
}

impl ChannelEvent {
    /// Creates a new inbound event from channel/session/user message.
    pub fn new(
        channel_id: ChannelId,
        session_id: SessionId,
        user_message: impl Into<String>,
    ) -> Self {
        Self {
            channel_id,
            session_id,
            user_message: user_message.into(),
            metadata: None,
        }
    }
}

/// Outbound response from the agent to a channel adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// Target channel identifier.
    pub channel_id: ChannelId,
    /// Session identifier associated with this response.
    pub session_id: SessionId,
    /// Response text payload.
    pub content: String,
    /// Whether this response represents an error message.
    pub is_error: bool,
}

impl AgentResponse {
    /// Creates a normal (non-error) agent response.
    pub fn new(channel_id: ChannelId, session_id: SessionId, content: impl Into<String>) -> Self {
        Self {
            channel_id,
            session_id,
            content: content.into(),
            is_error: false,
        }
    }

    /// Creates an error response.
    pub fn error(channel_id: ChannelId, session_id: SessionId, error: impl Into<String>) -> Self {
        Self {
            channel_id,
            session_id,
            content: error.into(),
            is_error: true,
        }
    }
}

/// Real-time progress events emitted during agent processing.
///
/// These events are sent via `tokio::sync::mpsc` from
/// `AgentRuntime::process_with_progress()` so that consumers (e.g. TUI)
/// can display live tool-call status while the ReAct loop runs.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// The LLM is being consulted (round N of the ReAct loop).
    LlmThinking { round: usize },
    /// A tool call has been dispatched but has not yet completed.
    ToolCallStarted {
        call_id: String,
        tool_name: String,
        args: serde_json::Value,
    },
    /// A tool call has finished executing.
    ToolCallFinished {
        call_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_event_new_initializes_without_metadata() {
        let channel_id = ChannelId::new("cli", "local");
        let session_id = SessionId::from("s1");
        let event = ChannelEvent::new(channel_id.clone(), session_id.clone(), "hello");

        assert_eq!(event.channel_id, channel_id);
        assert_eq!(event.session_id, session_id);
        assert_eq!(event.user_message, "hello");
        assert_eq!(event.metadata, None);
    }

    #[test]
    fn agent_response_new_is_not_error() {
        let channel_id = ChannelId::new("cli", "local");
        let session_id = SessionId::from("s1");
        let resp = AgentResponse::new(channel_id.clone(), session_id.clone(), "ok");

        assert_eq!(resp.channel_id, channel_id);
        assert_eq!(resp.session_id, session_id);
        assert_eq!(resp.content, "ok");
        assert!(!resp.is_error);
    }

    #[test]
    fn agent_response_error_sets_flag() {
        let channel_id = ChannelId::new("telegram", "99");
        let session_id = SessionId::from("s2");
        let resp = AgentResponse::error(channel_id, session_id, "boom");

        assert_eq!(resp.content, "boom");
        assert!(resp.is_error);
    }
}
