//! UiKeyTool — send keyboard shortcuts to macOS applications.

use async_trait::async_trait;
use proto::ToolResult;
use serde::Deserialize;

use crate::Tool;

#[derive(Debug, Deserialize)]
struct KeyArgs {
    keys: String,
    app: Option<String>,
}

/// Send a keyboard shortcut (e.g. "cmd+n", "cmd+shift+s") to the frontmost
/// or a specified application.
pub struct UiKeyTool;

impl UiKeyTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UiKeyTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiKeyTool {
    fn name(&self) -> &str {
        "ui.key"
    }

    fn description(&self) -> &str {
        "Send a keyboard shortcut to the frontmost or a specified macOS application. \
         Examples: 'cmd+n' for new document, 'cmd+shift+s' for save-as, 'return' for Enter."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "keys": {
                    "type": "string",
                    "description": "Key combo string, e.g. 'cmd+n', 'ctrl+shift+tab', 'return', 'escape'"
                },
                "app": {
                    "type": "string",
                    "description": "Target application name (optional, defaults to frontmost app)"
                }
            },
            "required": ["keys"]
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: KeyArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        #[cfg(target_os = "macos")]
        {
            let (key, modifiers) = match super::parse_key_combo(&parsed.keys) {
                Ok(v) => v,
                Err(e) => return ToolResult::error(call_id, self.name(), e),
            };

            let target = parsed.app.as_deref().unwrap_or("System Events");

            // Decide: keystroke (character) vs key code (special key)
            let key_action = if let Some(code) = super::key_name_to_code(&key) {
                if modifiers.is_empty() {
                    format!("key code {code}")
                } else {
                    format!("key code {code} using {modifiers}")
                }
            } else if key.len() == 1 {
                if modifiers.is_empty() {
                    format!(r#"keystroke "{key}""#)
                } else {
                    format!(r#"keystroke "{key}" using {modifiers}"#)
                }
            } else {
                return ToolResult::error(
                    call_id,
                    self.name(),
                    format!(
                        "Unknown key: '{key}'. Use single character or named key (return, escape, tab, etc.)"
                    ),
                );
            };

            let script = if target == "System Events" {
                format!(r#"tell application "System Events" to {key_action}"#)
            } else {
                format!(
                    r#"tell application "{target}" to activate
tell application "System Events" to {key_action}"#
                )
            };

            match super::run_osascript(&script, super::DEFAULT_TIMEOUT_SECS).await {
                Ok(_) => ToolResult::success(
                    call_id,
                    self.name(),
                    format!("Key '{}' sent to {}", parsed.keys, target),
                ),
                Err(e) => ToolResult::error(call_id, self.name(), e),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = parsed;
            ToolResult::error(call_id, self.name(), "ui.key is only supported on macOS")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_stable() {
        let tool = UiKeyTool::new();
        assert_eq!(tool.name(), "ui.key");
        assert!(tool.description().contains("keyboard"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["keys"].is_object());
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let tool = UiKeyTool::new();
        let result = tool.execute("c1", serde_json::json!({})).await;
        assert!(result.is_error);
    }
}
