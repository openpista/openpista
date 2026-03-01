//! TUI tool approval handler — prompts the user in the terminal.

use async_trait::async_trait;
use proto::{ToolApprovalDecision, ToolApprovalHandler, ToolApprovalRequest};
use tokio::sync::{mpsc, oneshot};

/// A pending approval request forwarded to the TUI event loop.
pub struct PendingApproval {
    /// The original approval request from the agent runtime.
    pub request: ToolApprovalRequest,
    /// Oneshot sender to deliver the user's decision back to the runtime.
    pub reply_tx: oneshot::Sender<ToolApprovalDecision>,
}

/// Tool approval handler for the TUI.
///
/// Forwards approval requests to the TUI event loop via an mpsc channel.
/// The event loop renders a prompt and sends the decision back through
/// a oneshot channel.
pub struct TuiApprovalHandler {
    tx: mpsc::Sender<PendingApproval>,
}

impl TuiApprovalHandler {
    /// Creates a new TUI approval handler and its receiver.
    ///
    /// The receiver should be polled in the TUI event loop.
    pub fn new() -> (Self, mpsc::Receiver<PendingApproval>) {
        let (tx, rx) = mpsc::channel(16);
        (Self { tx }, rx)
    }
}

#[async_trait]
impl ToolApprovalHandler for TuiApprovalHandler {
    async fn request_approval(&self, req: ToolApprovalRequest) -> ToolApprovalDecision {
        let (reply_tx, reply_rx) = oneshot::channel();
        let pending = PendingApproval {
            request: req,
            reply_tx,
        };

        if self.tx.send(pending).await.is_err() {
            // TUI event loop dropped — reject
            return ToolApprovalDecision::Reject;
        }

        reply_rx.await.unwrap_or(ToolApprovalDecision::Reject)
    }
}
