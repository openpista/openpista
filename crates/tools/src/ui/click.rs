//! UiClickTool — click at screen coordinates on macOS.

use async_trait::async_trait;
use proto::ToolResult;
use serde::Deserialize;

use crate::Tool;

#[derive(Debug, Deserialize)]
struct ClickArgs {
    x: f64,
    y: f64,
    #[serde(default = "default_click_type")]
    click_type: String,
}

fn default_click_type() -> String {
    "single".to_string()
}

/// Click at specific screen coordinates using JXA CGEvent posting.
pub struct UiClickTool;

impl UiClickTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UiClickTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiClickTool {
    fn name(&self) -> &str {
        "ui.click"
    }

    fn description(&self) -> &str {
        "Click at specific screen coordinates on macOS. \
         Supports single click, double click, and right click. \
         Use ui.snapshot first to identify element coordinates."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "number",
                    "description": "X coordinate (pixels from left)"
                },
                "y": {
                    "type": "number",
                    "description": "Y coordinate (pixels from top)"
                },
                "click_type": {
                    "type": "string",
                    "enum": ["single", "double", "right"],
                    "description": "Click type (default: single)"
                }
            },
            "required": ["x", "y"]
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: ClickArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        #[cfg(target_os = "macos")]
        {
            let x = parsed.x;
            let y = parsed.y;

            let script = match parsed.click_type.as_str() {
                "double" => format!(
                    r#"
ObjC.import('CoreGraphics');
var point = $.CGPointMake({x}, {y});
var down = $.CGEventCreateMouseEvent(null, $.kCGEventLeftMouseDown, point, $.kCGMouseButtonLeft);
var up = $.CGEventCreateMouseEvent(null, $.kCGEventLeftMouseUp, point, $.kCGMouseButtonLeft);
$.CGEventSetIntegerValueField(down, $.kCGMouseEventClickState, 2);
$.CGEventSetIntegerValueField(up, $.kCGMouseEventClickState, 2);
$.CGEventPost($.kCGHIDEventTap, down);
$.CGEventPost($.kCGHIDEventTap, up);
"double click at ({x}, {y})"
"#
                ),
                "right" => format!(
                    r#"
ObjC.import('CoreGraphics');
var point = $.CGPointMake({x}, {y});
var down = $.CGEventCreateMouseEvent(null, $.kCGEventRightMouseDown, point, $.kCGMouseButtonRight);
var up = $.CGEventCreateMouseEvent(null, $.kCGEventRightMouseUp, point, $.kCGMouseButtonRight);
$.CGEventPost($.kCGHIDEventTap, down);
$.CGEventPost($.kCGHIDEventTap, up);
"right click at ({x}, {y})"
"#
                ),
                _ => format!(
                    r#"
ObjC.import('CoreGraphics');
var point = $.CGPointMake({x}, {y});
var down = $.CGEventCreateMouseEvent(null, $.kCGEventLeftMouseDown, point, $.kCGMouseButtonLeft);
var up = $.CGEventCreateMouseEvent(null, $.kCGEventLeftMouseUp, point, $.kCGMouseButtonLeft);
$.CGEventPost($.kCGHIDEventTap, down);
$.CGEventPost($.kCGHIDEventTap, up);
"click at ({x}, {y})"
"#
                ),
            };

            match super::run_jxa(&script, super::DEFAULT_TIMEOUT_SECS).await {
                Ok(output) => ToolResult::success(call_id, self.name(), output),
                Err(e) => ToolResult::error(call_id, self.name(), e),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = parsed;
            ToolResult::error(call_id, self.name(), "ui.click is only supported on macOS")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_stable() {
        let tool = UiClickTool::new();
        assert_eq!(tool.name(), "ui.click");
        assert!(tool.description().contains("coordinates"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["x"].is_object());
        assert!(schema["properties"]["y"].is_object());
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let tool = UiClickTool::new();
        let result = tool.execute("c1", serde_json::json!({})).await;
        assert!(result.is_error);
    }
}
