//! Tool registry used by the runtime to list and execute tools.

use std::collections::HashMap;
use std::sync::Arc;

use proto::{ToolDefinition, ToolResult};
use tools::Tool;
use tracing::debug;

/// Registry of available tools
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Creates an empty tool registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: impl Tool + 'static) {
        let name = tool.name().to_string();
        debug!("Registering tool: {name}");
        self.tools.insert(name, Arc::new(tool));
    }

    /// Get tool definitions for the LLM
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|t| ToolDefinition::new(t.name(), t.description(), t.parameters_schema()))
            .collect()
    }

    /// Execute a tool call
    pub async fn execute(&self, call_id: &str, name: &str, args: serde_json::Value) -> ToolResult {
        if let Some(tool) = self.tools.get(name) {
            debug!("Executing tool: {name} (call_id: {call_id})");
            tool.execute(call_id, args).await
        } else {
            ToolResult::error(call_id, name, format!("Tool '{name}' not found"))
        }
    }

    /// Returns the list of registered tool names.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use proto::ToolResult;

    use super::*;

    struct EchoTool;

    #[async_trait]
    impl tools::Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echoes the input"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type":"object",
                "properties":{"value":{"type":"string"}},
                "required":["value"]
            })
        }

        async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
            let value = args["value"].as_str().unwrap_or_default().to_string();
            ToolResult::success(call_id, self.name(), value)
        }
    }

    #[tokio::test]
    async fn register_and_execute_known_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let result = registry
            .execute("c1", "echo", serde_json::json!({"value":"hello"}))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.output, "hello");
        assert_eq!(result.tool_name, "echo");
    }

    #[tokio::test]
    async fn execute_unknown_tool_returns_error() {
        let registry = ToolRegistry::new();
        let result = registry
            .execute("c2", "missing", serde_json::json!({}))
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }

    #[test]
    fn definitions_and_names_include_registered_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let names = registry.tool_names();
        assert_eq!(names, vec!["echo"]);

        let defs = registry.definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "echo");
        assert_eq!(defs[0].description, "Echoes the input");
        assert_eq!(defs[0].parameters["required"][0], "value");
    }
}
