//! DelegateTool — delegate a task to a sub-agent for autonomous execution.

use std::sync::Arc;

use async_trait::async_trait;
use proto::ToolResult;
use serde::Deserialize;

use crate::Tool;

/// Trait for executing sub-agent tasks. Implemented in the `agent` crate
/// to break circular dependency (tools → agent).
#[async_trait]
pub trait SubAgentExecutor: Send + Sync {
    /// Execute a task in a child agent context.
    async fn execute_sub_task(
        &self,
        task: &str,
        context: Option<&str>,
        max_rounds: usize,
        current_depth: u32,
        progress_tx: Option<tokio::sync::mpsc::Sender<proto::ProgressEvent>>,
    ) -> ToolResult;
}

#[derive(Debug, Deserialize)]
struct DelegateArgs {
    task: String,
    context: Option<String>,
    #[serde(default = "default_max_rounds")]
    max_rounds: usize,
}

fn default_max_rounds() -> usize {
    10
}

/// Delegate a complex task to a sub-agent that runs its own ReAct loop.
pub struct DelegateTool {
    executor: Arc<dyn SubAgentExecutor>,
    current_depth: u32,
    max_depth: u32,
}

impl DelegateTool {
    /// Create a new delegate tool.
    ///
    /// - `executor`: the sub-agent executor implementation
    /// - `current_depth`: nesting depth of the current agent (0 = root)
    /// - `max_depth`: maximum allowed nesting depth
    pub fn new(executor: Arc<dyn SubAgentExecutor>, current_depth: u32, max_depth: u32) -> Self {
        Self {
            executor,
            current_depth,
            max_depth,
        }
    }
}

#[async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "agent.delegate"
    }

    fn description(&self) -> &str {
        "Delegate a complex, multi-step task to a sub-agent. \
         The sub-agent runs its own ReAct loop with access to all tools. \
         Use this when a task requires multiple tool calls that can be handled independently. \
         The sub-agent will return the final result when complete."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Clear description of the task to delegate"
                },
                "context": {
                    "type": "string",
                    "description": "Optional additional context for the sub-agent"
                },
                "max_rounds": {
                    "type": "integer",
                    "description": "Maximum ReAct loop rounds for the sub-agent (default: 10)"
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: DelegateArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        if self.current_depth >= self.max_depth {
            return ToolResult::error(
                call_id,
                self.name(),
                format!(
                    "Maximum sub-agent depth ({}) reached. Cannot delegate further.",
                    self.max_depth
                ),
            );
        }

        self.executor
            .execute_sub_task(
                &parsed.task,
                parsed.context.as_deref(),
                parsed.max_rounds.min(20),
                self.current_depth + 1,
                None,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockExecutor;

    #[async_trait]
    impl SubAgentExecutor for MockExecutor {
        async fn execute_sub_task(
            &self,
            task: &str,
            _context: Option<&str>,
            _max_rounds: usize,
            _current_depth: u32,
            _progress_tx: Option<tokio::sync::mpsc::Sender<proto::ProgressEvent>>,
        ) -> ToolResult {
            ToolResult::success("mock", "agent.delegate", format!("Completed: {task}"))
        }
    }

    #[test]
    fn metadata_is_stable() {
        let tool = DelegateTool::new(Arc::new(MockExecutor), 0, 3);
        assert_eq!(tool.name(), "agent.delegate");
        assert!(tool.description().contains("sub-agent"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["task"].is_object());
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let tool = DelegateTool::new(Arc::new(MockExecutor), 0, 3);
        let result = tool.execute("c1", serde_json::json!({})).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn rejects_at_max_depth() {
        let tool = DelegateTool::new(Arc::new(MockExecutor), 3, 3);
        let result = tool
            .execute("c1", serde_json::json!({"task": "test"}))
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("Maximum sub-agent depth"));
    }

    #[tokio::test]
    async fn delegates_to_executor() {
        let tool = DelegateTool::new(Arc::new(MockExecutor), 0, 3);
        let result = tool
            .execute("c1", serde_json::json!({"task": "do something"}))
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("Completed: do something"));
    }
}
