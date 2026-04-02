//! UiSnapshotTool — read the accessibility tree of a macOS application.

use async_trait::async_trait;
use proto::ToolResult;
use serde::Deserialize;

use crate::Tool;

#[derive(Debug, Deserialize)]
struct SnapshotArgs {
    app: Option<String>,
    #[serde(default = "default_depth")]
    depth: u32,
}

fn default_depth() -> u32 {
    3
}

const MAX_OUTPUT_CHARS: usize = 12_000;

/// Read the UI accessibility tree of a macOS application as JSON.
/// Useful for understanding the current state of an app's UI before
/// performing click or keyboard actions.
pub struct UiSnapshotTool;

impl UiSnapshotTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UiSnapshotTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiSnapshotTool {
    fn name(&self) -> &str {
        "ui.snapshot"
    }

    fn description(&self) -> &str {
        "Read the accessibility tree of a macOS application as JSON. \
         Returns UI element hierarchy with roles, titles, positions, and sizes. \
         Use this to understand the current UI state before clicking or typing."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "app": {
                    "type": "string",
                    "description": "Application name to inspect (default: frontmost app)"
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum depth of the UI tree to traverse (default: 3, max: 6)"
                }
            }
        })
    }

    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
        let parsed: SnapshotArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(call_id, self.name(), format!("Invalid arguments: {e}"));
            }
        };

        #[cfg(target_os = "macos")]
        {
            let depth = parsed.depth.min(6);

            let script = format!(
                r#"
ObjC.import('stdlib');

var se = Application('System Events');
se.includeStandardAdditions = true;

function getAppProcess() {{
    {app_target_fn}
}}

function dumpElement(el, depth, maxDepth) {{
    if (depth > maxDepth) return null;
    var result = {{}};
    try {{ result.role = el.role(); }} catch(e) {{}}
    try {{ result.title = el.title(); }} catch(e) {{}}
    try {{ result.name = el.name(); }} catch(e) {{}}
    try {{ result.value = String(el.value()); }} catch(e) {{}}
    try {{ result.description = el.description(); }} catch(e) {{}}
    try {{
        var pos = el.position();
        var sz = el.size();
        result.position = {{ x: pos[0], y: pos[1] }};
        result.size = {{ width: sz[0], height: sz[1] }};
    }} catch(e) {{}}
    try {{
        var children = el.uiElements();
        if (children.length > 0 && depth < maxDepth) {{
            result.children = [];
            for (var i = 0; i < Math.min(children.length, 50); i++) {{
                var child = dumpElement(children[i], depth + 1, maxDepth);
                if (child) result.children.push(child);
            }}
        }} else if (children.length > 0) {{
            result.childCount = children.length;
        }}
    }} catch(e) {{}}
    return result;
}}

var proc = getAppProcess();
var windows = proc.windows();
var result = [];
for (var i = 0; i < Math.min(windows.length, 5); i++) {{
    result.push(dumpElement(windows[i], 0, {depth}));
}}
JSON.stringify(result, null, 2);
"#,
                app_target_fn = if let Some(ref app) = parsed.app {
                    format!(r#"return se.applicationProcesses.byName("{app}"); "#)
                } else {
                    "return se.applicationProcesses.whose({frontmost: true})[0];".to_string()
                },
                depth = depth,
            );

            match super::run_jxa(&script, 30).await {
                Ok(output) => {
                    let truncated = if output.chars().count() > MAX_OUTPUT_CHARS {
                        let t: String = output.chars().take(MAX_OUTPUT_CHARS).collect();
                        format!("{t}\n[... output truncated at {MAX_OUTPUT_CHARS} chars]")
                    } else {
                        output
                    };
                    ToolResult::success(call_id, self.name(), truncated)
                }
                Err(e) => ToolResult::error(call_id, self.name(), e),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = parsed;
            ToolResult::error(
                call_id,
                self.name(),
                "ui.snapshot is only supported on macOS",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_stable() {
        let tool = UiSnapshotTool::new();
        assert_eq!(tool.name(), "ui.snapshot");
        assert!(tool.description().contains("accessibility"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["app"].is_object());
        assert!(schema["properties"]["depth"].is_object());
    }

    #[tokio::test]
    async fn rejects_invalid_args() {
        let tool = UiSnapshotTool::new();
        let result = tool
            .execute("c1", serde_json::json!({"depth": "bad"}))
            .await;
        assert!(result.is_error);
    }
}
