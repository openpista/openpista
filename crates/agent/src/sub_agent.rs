//! RuntimeSubAgentExecutor — executes sub-agent tasks using a shared AgentRuntime.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use proto::{ChannelId, SessionId, ToolResult};
use tokio::time::timeout;
use tools::SubAgentExecutor;
use tracing::{info, warn};

use crate::AgentRuntime;

const SUB_AGENT_TIMEOUT_SECS: u64 = 120;

/// Executes sub-agent tasks by creating a child session on the shared AgentRuntime.
pub struct RuntimeSubAgentExecutor {
    runtime: Arc<AgentRuntime>,
}

impl RuntimeSubAgentExecutor {
    /// Creates a new sub-agent executor backed by the given runtime.
    pub fn new(runtime: Arc<AgentRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl SubAgentExecutor for RuntimeSubAgentExecutor {
    async fn execute_sub_task(
        &self,
        task: &str,
        context: Option<&str>,
        max_rounds: usize,
        current_depth: u32,
        progress_tx: Option<tokio::sync::mpsc::Sender<proto::ProgressEvent>>,
    ) -> ToolResult {
        let session_id = SessionId::from(format!("subagent:{}", uuid::Uuid::new_v4()));
        let channel_id = ChannelId::new("cli", "subagent");

        // Build the user message for the sub-agent
        let user_message = if let Some(ctx) = context {
            format!("Context: {ctx}\n\nTask: {task}")
        } else {
            task.to_string()
        };

        info!(
            depth = current_depth,
            session = %session_id,
            "Sub-agent starting: {:.80}",
            task
        );

        // Emit SubAgentStarted progress event
        if let Some(ref tx) = progress_tx {
            let _ = tx.try_send(proto::ProgressEvent::SubAgentStarted {
                task: task.chars().take(100).collect(),
                depth: current_depth,
            });
        }

        // Run with timeout
        let result = timeout(Duration::from_secs(SUB_AGENT_TIMEOUT_SECS), async {
            if let Some(tx) = progress_tx.clone() {
                self.runtime
                    .process_with_progress(&channel_id, &session_id, &user_message, None, tx)
                    .await
            } else {
                // Use the version without progress if no tx provided
                self.runtime
                    .process(&channel_id, &session_id, &user_message, None)
                    .await
                    .map(|(text, _usage)| text)
            }
        })
        .await;

        // Emit SubAgentFinished progress event
        if let Some(ref tx) = progress_tx {
            let is_error = matches!(&result, Ok(Err(_)) | Err(_));
            let _ = tx.try_send(proto::ProgressEvent::SubAgentFinished {
                depth: current_depth,
                is_error,
            });
        }

        let _ = max_rounds; // max_rounds is handled by runtime's own max_tool_rounds

        match result {
            Ok(Ok(text)) => {
                info!(
                    depth = current_depth,
                    session = %session_id,
                    "Sub-agent completed: {:.80}",
                    text
                );
                ToolResult::success("subagent", "agent.delegate", text)
            }
            Ok(Err(e)) => {
                warn!(
                    depth = current_depth,
                    session = %session_id,
                    "Sub-agent error: {e}"
                );
                ToolResult::error(
                    "subagent",
                    "agent.delegate",
                    format!("Sub-agent error: {e}"),
                )
            }
            Err(_) => {
                warn!(
                    depth = current_depth,
                    session = %session_id,
                    "Sub-agent timed out after {SUB_AGENT_TIMEOUT_SECS}s"
                );
                ToolResult::error(
                    "subagent",
                    "agent.delegate",
                    format!("Sub-agent timed out after {SUB_AGENT_TIMEOUT_SECS}s"),
                )
            }
        }
    }
}
