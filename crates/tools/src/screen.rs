//! Screen capture tool.

use async_trait::async_trait;
#[cfg(not(target_env = "musl"))]
use base64::{Engine as _, engine::general_purpose};
#[cfg(not(target_env = "musl"))]
use image::ImageFormat;
use proto::ToolResult;
#[cfg(not(target_env = "musl"))]
use screenshots::Screen;
use serde::Deserialize;
#[cfg(not(target_env = "musl"))]
use std::io::Cursor;

use crate::Tool;

#[derive(Debug, Deserialize)]
struct ScreenArgs {
    display: Option<usize>,
}

/// Screen capture tool.
pub struct ScreenTool;

impl ScreenTool {
    /// Creates a new screen tool instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ScreenTool {
    /// Creates a default screen tool instance.
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ScreenTool {
    fn name(&self) -> &str {
        "screen.capture"
    }

    fn description(&self) -> &str {
        "Capture a screenshot and return PNG bytes as base64"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "display": {
                    "type": "integer",
                    "description": "Display index to capture (default: 0)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let screen_args: ScreenArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        match capture_png_base64(screen_args.display.unwrap_or(0)) {
            Ok(output) => ToolResult::success(call_id, self.name(), output),
            Err(e) => ToolResult::error(call_id, self.name(), e),
        }
    }
}

#[cfg(not(target_env = "musl"))]
fn capture_png_base64(display_index: usize) -> Result<String, String> {
    let screens = Screen::all().map_err(|e| format!("Failed to enumerate displays: {e}"))?;
    if screens.is_empty() {
        return Err("No displays found".to_string());
    }

    let screen = screens
        .get(display_index)
        .ok_or_else(|| format!("Display index out of range: {display_index}"))?;

    let captured = screen
        .capture()
        .map_err(|e| format!("Screen capture failed: {e}"))?;

    let width = captured.width();
    let height = captured.height();

    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(captured)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|e| format!("Failed to encode PNG: {e}"))?;

    let data_b64 = general_purpose::STANDARD.encode(&png);

    let output = serde_json::json!({
        "mime": "image/png",
        "display": display_index,
        "width": width,
        "height": height,
        "size_bytes": png.len(),
        "data_b64": data_b64,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

#[cfg(target_env = "musl")]
fn capture_png_base64(_display_index: usize) -> Result<String, String> {
    Err("screen.capture is not supported on musl targets".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_tool_metadata_is_stable() {
        let tool = ScreenTool::new();
        assert_eq!(tool.name(), "screen.capture");
        assert!(tool.description().contains("base64"));
        assert_eq!(tool.parameters_schema()["type"], "object");
    }

    #[tokio::test]
    async fn execute_rejects_invalid_arguments() {
        let tool = ScreenTool::new();
        let result = tool
            .execute("call-2", serde_json::json!({"display":"bad"}))
            .await;
        assert_eq!(result.call_id, "call-2");
        assert_eq!(result.tool_name, "screen.capture");
        assert!(result.is_error);
        assert!(result.output.contains("Invalid arguments"));
    }

    #[cfg(not(target_env = "musl"))]
    #[tokio::test]
    async fn execute_with_valid_arguments_returns_result_shape() {
        let tool = ScreenTool::new();
        let result = tool
            .execute("call-3", serde_json::json!({"display":0}))
            .await;
        assert_eq!(result.call_id, "call-3");
        assert_eq!(result.tool_name, "screen.capture");
        assert!(!result.output.is_empty());
    }

    #[cfg(target_env = "musl")]
    #[tokio::test]
    async fn execute_with_valid_arguments_returns_result_shape() {
        let tool = ScreenTool::new();
        let result = tool
            .execute("call-3", serde_json::json!({"display":0}))
            .await;
        assert_eq!(result.call_id, "call-3");
        assert_eq!(result.tool_name, "screen.capture");
        assert!(result.is_error);
        assert!(
            result
                .output
                .contains("screen.capture is not supported on musl targets")
        );
    }

    #[test]
    fn capture_png_base64_handles_out_of_range_or_missing_display() {
        let result = capture_png_base64(usize::MAX);
        assert!(result.is_err());
        if let Err(err) = result {
            assert!(
                err.contains("No displays found")
                    || err.contains("Display index out of range")
                    || err.contains("screen.capture is not supported on musl targets")
                    || err.contains("Failed to enumerate displays")
            );
        }
    }
}
