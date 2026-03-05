//! Bash tool implementation.

use async_trait::async_trait;
use proto::ToolResult;
use serde::Deserialize;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::Tool;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_OUTPUT_CHARS: usize = 10_000;

#[derive(Debug, Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    working_dir: Option<String>,
}

/// Tool that executes bash commands
pub struct BashTool {
    default_timeout: Duration,
}

impl BashTool {
    /// Creates a bash tool with the default timeout.
    pub fn new() -> Self {
        Self {
            default_timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Creates a bash tool with a custom default timeout in seconds.
    pub fn with_timeout(secs: u64) -> Self {
        Self {
            default_timeout: Duration::from_secs(secs),
        }
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "system.run"
    }

    fn description(&self) -> &str {
        "Execute a bash command and return stdout, stderr, and exit code. \
         Use for file operations, system commands, running scripts, etc. \
         Output is limited to 10,000 characters. Timeout is 30 seconds by default."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 300)"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory for the command (optional)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let bash_args: BashArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        let timeout_duration = bash_args
            .timeout_secs
            .map(|s| Duration::from_secs(s.min(300)))
            .unwrap_or(self.default_timeout);

        debug!("Executing bash command: {}", bash_args.command);

        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(&bash_args.command);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if let Some(dir) = &bash_args.working_dir {
            cmd.current_dir(dir);
        }

        let result = timeout(timeout_duration, async {
            match cmd.output().await {
                Ok(output) => {
                    let stdout = crate::util::truncate_str(
                        &String::from_utf8_lossy(&output.stdout),
                        MAX_OUTPUT_CHARS / 2,
                    );
                    let stderr = crate::util::truncate_str(
                        &String::from_utf8_lossy(&output.stderr),
                        MAX_OUTPUT_CHARS / 2,
                    );
                    let exit_code = output.status.code().unwrap_or(-1);
                    Ok((stdout, stderr, exit_code))
                }
                Err(e) => Err(e.to_string()),
            }
        })
        .await;

        match result {
            Ok(Ok((stdout, stderr, exit_code))) => {
                let output = crate::util::format_output(&stdout, &stderr, exit_code as i64);
                if exit_code == 0 {
                    ToolResult::success(call_id, self.name(), output)
                } else {
                    // Non-zero exit is not an error per se — let the LLM decide
                    ToolResult::success(call_id, self.name(), output)
                }
            }
            Ok(Err(e)) => ToolResult::error(call_id, self.name(), format!("Execution failed: {e}")),
            Err(_) => {
                warn!(
                    "Bash command timed out after {}s: {}",
                    timeout_duration.as_secs(),
                    bash_args.command
                );
                ToolResult::error(
                    call_id,
                    self.name(),
                    format!("Command timed out after {}s", timeout_duration.as_secs()),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_returns_invalid_arguments_error() {
        let tool = BashTool::new();
        let result = tool
            .execute("c1", serde_json::json!({"timeout_secs": 1}))
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[tokio::test]
    async fn execute_runs_successful_command() {
        let tool = BashTool::new();
        let result = tool
            .execute("c2", serde_json::json!({"command":"printf 'hello'"}))
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("stdout:\nhello"));
        assert!(result.output.contains("exit_code: 0"));
    }

    #[tokio::test]
    async fn execute_keeps_non_zero_exit_as_success_result() {
        let tool = BashTool::new();
        let result = tool
            .execute("c3", serde_json::json!({"command":"echo err 1>&2; exit 7"}))
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("stderr:\nerr"));
        assert!(result.output.contains("exit_code: 7"));
    }

    #[tokio::test]
    async fn execute_honors_timeout() {
        let tool = BashTool::with_timeout(1);
        let result = tool
            .execute("c4", serde_json::json!({"command":"sleep 2"}))
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("timed out"));
    }

    #[tokio::test]
    async fn execute_uses_working_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let tool = BashTool::new();
        let result = tool
            .execute(
                "c5",
                serde_json::json!({
                    "command":"pwd",
                    "working_dir": dir.path().to_string_lossy()
                }),
            )
            .await;

        assert!(!result.is_error);
        assert!(
            result
                .output
                .contains(&dir.path().to_string_lossy().to_string())
        );
        assert!(result.output.contains("exit_code: 0"));
    }

    #[test]
    fn bash_tool_default_matches_new() {
        let a = BashTool::new();
        let b = BashTool::default();
        assert_eq!(a.default_timeout, b.default_timeout);
    }

    #[test]
    fn bash_tool_with_timeout_stores_custom_duration() {
        let tool = BashTool::with_timeout(120);
        assert_eq!(tool.default_timeout, Duration::from_secs(120));
    }

    #[test]
    fn bash_tool_metadata_is_stable() {
        let tool = BashTool::new();
        assert_eq!(tool.name(), "system.run");
        assert!(tool.description().contains("bash"));
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["command"].is_object());
        assert!(schema["properties"]["timeout_secs"].is_object());
        assert!(schema["properties"]["working_dir"].is_object());
    }

    #[tokio::test]
    async fn execute_with_large_timeout_succeeds() {
        let tool = BashTool::new();
        // Large timeout values should not break successful command execution.
        let result = tool
            .execute(
                "c6",
                serde_json::json!({"command": "echo ok", "timeout_secs": 999}),
            )
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("exit_code: 0"));
    }
}
