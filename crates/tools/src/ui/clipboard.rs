//! UiClipboardTool — read from or write to the macOS clipboard.

use async_trait::async_trait;
use proto::ToolResult;
use serde::Deserialize;
use tokio::process::Command;

use crate::Tool;

#[derive(Debug, Deserialize)]
struct ClipboardArgs {
    #[serde(default = "default_action")]
    action: String,
    content: Option<String>,
}

fn default_action() -> String {
    "read".to_string()
}

/// Read from or write to the system clipboard.
pub struct UiClipboardTool;

impl UiClipboardTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UiClipboardTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiClipboardTool {
    fn name(&self) -> &str {
        "ui.clipboard"
    }

    fn description(&self) -> &str {
        "Read from or write to the system clipboard. \
         Use action 'read' to get current clipboard contents, \
         or 'write' with 'content' to set clipboard text."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["read", "write"],
                    "description": "Action to perform (default: read)"
                },
                "content": {
                    "type": "string",
                    "description": "Text to write to clipboard (required when action is 'write')"
                }
            }
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: ClipboardArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        #[cfg(target_os = "macos")]
        {
            match parsed.action.as_str() {
                "write" => {
                    let content = match &parsed.content {
                        Some(c) => c,
                        None => {
                            return ToolResult::error(
                                call_id,
                                self.name(),
                                "'content' is required when action is 'write'",
                            );
                        }
                    };

                    let mut child = match Command::new("pbcopy")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                    {
                        Ok(c) => c,
                        Err(e) => {
                            return ToolResult::error(
                                call_id,
                                self.name(),
                                format!("Failed to spawn pbcopy: {e}"),
                            );
                        }
                    };

                    if let Some(mut stdin) = child.stdin.take() {
                        use tokio::io::AsyncWriteExt;
                        if let Err(e) = stdin.write_all(content.as_bytes()).await {
                            return ToolResult::error(
                                call_id,
                                self.name(),
                                format!("Failed to write to pbcopy: {e}"),
                            );
                        }
                        drop(stdin);
                    }

                    match child.wait().await {
                        Ok(status) if status.success() => ToolResult::success(
                            call_id,
                            self.name(),
                            format!("Clipboard set ({} chars)", content.len()),
                        ),
                        Ok(status) => ToolResult::error(
                            call_id,
                            self.name(),
                            format!("pbcopy exited with: {status}"),
                        ),
                        Err(e) => {
                            ToolResult::error(call_id, self.name(), format!("pbcopy error: {e}"))
                        }
                    }
                }
                _ => {
                    let output = Command::new("pbpaste").output().await;
                    match output {
                        Ok(out) if out.status.success() => {
                            let text = String::from_utf8_lossy(&out.stdout).to_string();
                            ToolResult::success(call_id, self.name(), text)
                        }
                        Ok(out) => ToolResult::error(
                            call_id,
                            self.name(),
                            format!("pbpaste error: {}", String::from_utf8_lossy(&out.stderr)),
                        ),
                        Err(e) => ToolResult::error(
                            call_id,
                            self.name(),
                            format!("Failed to run pbpaste: {e}"),
                        ),
                    }
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = parsed;
            ToolResult::error(
                call_id,
                self.name(),
                "ui.clipboard is only supported on macOS",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_stable() {
        let tool = UiClipboardTool::new();
        assert_eq!(tool.name(), "ui.clipboard");
        assert!(tool.description().contains("clipboard"));
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let tool = UiClipboardTool::new();
        let result = tool.execute("c1", serde_json::json!({"action": 123})).await;
        assert!(result.is_error);
    }
}
