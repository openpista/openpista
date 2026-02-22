//! Shared protocol types for channels, agent runtime, and tools.
//!
//! This crate defines serializable message/event/tool structures and
//! strongly-typed error enums shared across the workspace.

pub mod error;
pub mod event;
pub mod message;
pub mod tool;

/// Re-export of all protocol error types.
pub use error::*;
/// Re-export of inbound/outbound event types.
pub use event::{
    AgentResponse, ChannelEvent, ProgressEvent, WORKER_REPORT_KIND, WorkerOutput, WorkerReport,
};
/// Re-export of conversation/message identity types.
pub use message::{AgentMessage, ChannelId, Role, SessionId};
/// Re-export of tool call definition and result types.
pub use tool::{ToolCall, ToolDefinition, ToolResult};

/// Returns `true` when the key looks like an Anthropic OAuth access token
/// (`sk-ant-oat*`) rather than a permanent API key (`sk-ant-api*`).
///
/// Anthropic's Messages API does not accept OAuth tokens — only permanent
/// API keys sent via the `x-api-key` header. This helper is used across
/// crates to reject OAuth tokens early with a clear error message.
pub fn is_anthropic_oauth_token(key: &str) -> bool {
    key.starts_with("sk-ant-oat")
}
