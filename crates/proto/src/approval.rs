//! Tool call approval types shared across channels and the agent runtime.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// User's decision on a tool call approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolApprovalDecision {
    /// Approve this single tool call.
    Approve,
    /// Reject this tool call.
    Reject,
    /// Approve and allow all calls to this tool for the rest of the session.
    AllowForSession,
}

/// A request for user approval before executing a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolApprovalRequest {
    /// Tool-call identifier from the LLM.
    pub call_id: String,
    /// Name of the tool to be executed.
    pub tool_name: String,
    /// JSON arguments for the tool call.
    pub arguments: serde_json::Value,
}

/// Handler for tool call approval requests.
///
/// Each channel implements this trait to ask users for approval before
/// executing a tool call. The agent runtime calls [`request_approval`](Self::request_approval)
/// before every tool execution unless the tool is already approved for the session.
#[async_trait]
pub trait ToolApprovalHandler: Send + Sync {
    /// Request approval for a tool call.
    ///
    /// Returns the user's decision. Implementations should present the request
    /// to the user and wait for their response.
    async fn request_approval(&self, req: ToolApprovalRequest) -> ToolApprovalDecision;
}

/// Auto-approve handler that approves all tool calls without asking.
///
/// Used as the default when no interactive approval is configured (e.g. Telegram,
/// CLI `run` mode, or tests).
pub struct AutoApproveHandler;

#[async_trait]
impl ToolApprovalHandler for AutoApproveHandler {
    async fn request_approval(&self, _req: ToolApprovalRequest) -> ToolApprovalDecision {
        ToolApprovalDecision::Approve
    }
}
