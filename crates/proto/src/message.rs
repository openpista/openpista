use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a session
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    /// Creates a new random session identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Returns the raw session identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Unique identifier for a channel (e.g., "telegram:12345", "cli:local")
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub String);

impl ChannelId {
    /// Builds a channel identifier from adapter name and adapter-specific id.
    pub fn new(adapter: &str, id: &str) -> Self {
        Self(format!("{adapter}:{id}"))
    }

    /// Returns the raw channel identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ChannelId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ChannelId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Message role in a conversation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Message authored by an end user.
    User,
    /// Message authored by the assistant/agent.
    Assistant,
    /// System-level instruction message.
    System,
    /// Tool execution result message.
    Tool,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::System => write!(f, "system"),
            Role::Tool => write!(f, "tool"),
        }
    }
}

impl std::str::FromStr for Role {
    type Err = crate::error::ProtoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Role::User),
            "assistant" => Ok(Role::Assistant),
            "system" => Ok(Role::System),
            "tool" => Ok(Role::Tool),
            other => Err(crate::error::ProtoError::InvalidRole(other.to_string())),
        }
    }
}

/// A message in an agent conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    /// Unique message id.
    pub id: String,
    /// Session that owns this message.
    pub session_id: SessionId,
    /// Semantic role of this message.
    pub role: Role,
    /// Message content payload.
    pub content: String,
    /// Tool call id when role is `Tool`.
    pub tool_call_id: Option<String>,
    /// Tool name when role is `Tool`.
    pub tool_name: Option<String>,
    /// Assistant tool calls when role is `Assistant`.
    pub tool_calls: Option<Vec<crate::tool::ToolCall>>,
    /// Message creation timestamp in UTC.
    pub created_at: DateTime<Utc>,
}

impl AgentMessage {
    /// Creates a regular conversation message for the given session/role.
    pub fn new(session_id: SessionId, role: Role, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session_id,
            role,
            content: content.into(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: None,
            created_at: Utc::now(),
        }
    }

    /// Creates an assistant message containing tool calls.
    pub fn assistant_tool_calls(
        session_id: SessionId,
        tool_calls: Vec<crate::tool::ToolCall>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session_id,
            role: Role::Assistant,
            content: String::new(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Some(tool_calls),
            created_at: Utc::now(),
        }
    }

    /// Creates a tool result message for the given tool call.
    pub fn tool_result(
        session_id: SessionId,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session_id,
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
            tool_calls: None,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::ProtoError;

    #[test]
    fn session_id_new_creates_non_empty_value() {
        let session = SessionId::new();
        assert!(!session.as_str().is_empty());
    }

    #[test]
    fn channel_id_new_formats_adapter_and_id() {
        let channel = ChannelId::new("telegram", "1234");
        assert_eq!(channel.as_str(), "telegram:1234");
    }

    #[test]
    fn role_display_and_parse_round_trip() {
        let roles = [Role::User, Role::Assistant, Role::System, Role::Tool];
        for role in roles {
            let rendered = role.to_string();
            let parsed = Role::from_str(&rendered).expect("role should parse");
            assert_eq!(parsed, role);
        }
    }

    #[test]
    fn role_parse_invalid_value_returns_error() {
        let err = Role::from_str("owner").expect_err("invalid role should fail");
        match err {
            ProtoError::InvalidRole(value) => assert_eq!(value, "owner"),
            other => panic!("unexpected error variant: {other}"),
        }
    }

    #[test]
    fn agent_message_new_sets_common_fields() {
        let session = SessionId::from("session-1");
        let msg = AgentMessage::new(session.clone(), Role::User, "hello");

        assert!(!msg.id.is_empty());
        assert_eq!(msg.session_id, session);
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.tool_call_id, None);
        assert_eq!(msg.tool_name, None);
        assert_eq!(msg.tool_calls, None);
    }

    #[test]
    fn agent_message_tool_result_sets_tool_metadata() {
        let session = SessionId::from("session-2");
        let msg = AgentMessage::tool_result(session.clone(), "call-1", "system.run", "ok");

        assert_eq!(msg.session_id, session);
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.content, "ok");
        assert_eq!(msg.tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(msg.tool_name.as_deref(), Some("system.run"));
        assert_eq!(msg.tool_calls, None);
    }

    #[test]
    fn assistant_tool_calls_sets_assistant_metadata() {
        let session = SessionId::from("session-3");
        let calls = vec![crate::tool::ToolCall::new(
            "system.run",
            serde_json::json!({"command":"echo hi"}),
        )];
        let msg = AgentMessage::assistant_tool_calls(session.clone(), calls.clone());

        assert_eq!(msg.session_id, session);
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "");
        assert_eq!(msg.tool_call_id, None);
        assert_eq!(msg.tool_name, None);
        assert_eq!(msg.tool_calls, Some(calls));
    }
}
