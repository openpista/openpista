//! UiAppTool — launch, activate, or quit macOS applications.

use async_trait::async_trait;
use proto::ToolResult;
use serde::Deserialize;

use crate::Tool;

#[derive(Debug, Deserialize)]
struct UiAppArgs {
    app_name: String,
    #[serde(default = "default_action")]
    action: String,
}

fn default_action() -> String {
    "open".to_string()
}

/// Launch, activate, or quit a macOS application by name.
pub struct UiAppTool;

impl UiAppTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UiAppTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiAppTool {
    fn name(&self) -> &str {
        "ui.app"
    }

    fn description(&self) -> &str {
        "Launch, activate, or quit a macOS application. \
         Use action 'open' or 'activate' to bring an app to the foreground, \
         or 'quit' to close it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "app_name": {
                    "type": "string",
                    "description": "The application name (e.g. \"Notes\", \"Safari\", \"Terminal\")"
                },
                "action": {
                    "type": "string",
                    "enum": ["open", "activate", "quit"],
                    "description": "Action to perform (default: open)"
                }
            },
            "required": ["app_name"]
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: UiAppArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        #[cfg(target_os = "macos")]
        {
            let script = match parsed.action.as_str() {
                "quit" => {
                    format!(r#"tell application "{}" to quit"#, parsed.app_name)
                }
                _ => {
                    format!(r#"tell application "{}" to activate"#, parsed.app_name)
                }
            };

            match super::run_osascript(&script, super::DEFAULT_TIMEOUT_SECS).await {
                Ok(output) => {
                    let msg = if output.is_empty() {
                        format!("{} {} successful", parsed.action, parsed.app_name)
                    } else {
                        output
                    };
                    ToolResult::success(call_id, self.name(), msg)
                }
                Err(e) => ToolResult::error(call_id, self.name(), e),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = parsed;
            ToolResult::error(call_id, self.name(), "ui.app is only supported on macOS")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_stable() {
        let tool = UiAppTool::new();
        assert_eq!(tool.name(), "ui.app");
        assert!(tool.description().contains("macOS"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["app_name"].is_object());
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let tool = UiAppTool::new();
        let result = tool.execute("c1", serde_json::json!({})).await;
        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }
}
