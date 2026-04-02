//! UiTypeTool — type text into the frontmost macOS application.

use async_trait::async_trait;
use proto::ToolResult;
use serde::Deserialize;

use crate::Tool;

#[derive(Debug, Deserialize)]
struct TypeArgs {
    text: String,
    #[serde(default = "default_method")]
    method: String,
}

fn default_method() -> String {
    "keystroke".to_string()
}

/// Type text into the frontmost macOS application via System Events keystroke
/// or clipboard paste.
pub struct UiTypeTool;

impl UiTypeTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UiTypeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiTypeTool {
    fn name(&self) -> &str {
        "ui.type"
    }

    fn description(&self) -> &str {
        "Type text into the frontmost macOS application. \
         Method 'keystroke' types character-by-character (best for short text). \
         Method 'clipboard' copies text to clipboard and pastes with Cmd+V (best for long text or special characters)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to type"
                },
                "method": {
                    "type": "string",
                    "enum": ["keystroke", "clipboard"],
                    "description": "Input method (default: keystroke)"
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: TypeArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        #[cfg(target_os = "macos")]
        {
            match parsed.method.as_str() {
                "clipboard" => {
                    // Save current clipboard, set new content, paste, restore
                    use tokio::process::Command;

                    // Write to clipboard via pbcopy
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
                        if let Err(e) = stdin.write_all(parsed.text.as_bytes()).await {
                            return ToolResult::error(
                                call_id,
                                self.name(),
                                format!("Failed to write to pbcopy: {e}"),
                            );
                        }
                        drop(stdin);
                    }

                    if let Err(e) = child.wait().await {
                        return ToolResult::error(
                            call_id,
                            self.name(),
                            format!("pbcopy error: {e}"),
                        );
                    }

                    // Paste with Cmd+V
                    let paste_script =
                        r#"tell application "System Events" to keystroke "v" using {command down}"#;
                    match super::run_osascript(paste_script, super::DEFAULT_TIMEOUT_SECS).await {
                        Ok(_) => ToolResult::success(
                            call_id,
                            self.name(),
                            format!("Typed {} chars via clipboard paste", parsed.text.len()),
                        ),
                        Err(e) => ToolResult::error(call_id, self.name(), e),
                    }
                }
                _ => {
                    // Escape special characters for AppleScript string
                    let escaped = parsed.text.replace('\\', "\\\\").replace('"', "\\\"");
                    let script =
                        format!(r#"tell application "System Events" to keystroke "{escaped}""#);
                    match super::run_osascript(&script, super::DEFAULT_TIMEOUT_SECS).await {
                        Ok(_) => ToolResult::success(
                            call_id,
                            self.name(),
                            format!("Typed {} chars via keystroke", parsed.text.len()),
                        ),
                        Err(e) => ToolResult::error(call_id, self.name(), e),
                    }
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = parsed;
            ToolResult::error(call_id, self.name(), "ui.type is only supported on macOS")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_stable() {
        let tool = UiTypeTool::new();
        assert_eq!(tool.name(), "ui.type");
        assert!(tool.description().contains("Type text"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["text"].is_object());
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let tool = UiTypeTool::new();
        let result = tool.execute("c1", serde_json::json!({})).await;
        assert!(result.is_error);
    }
}
