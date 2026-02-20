//! Tool trait and built-in tool implementations.
//!
//! The agent runtime uses this crate to expose executable capabilities
//! such as shell commands and (future) browser/screen integrations.

pub mod bash;
pub mod browser;
pub mod container;
pub mod screen;
mod wasm_runtime;

pub use bash::BashTool;
pub use browser::{BrowserClickTool, BrowserScreenshotTool, BrowserTool, BrowserTypeTool};
pub use container::ContainerTool;
pub use screen::ScreenTool;

use async_trait::async_trait;
use proto::ToolResult;

/// Trait that all tools must implement
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name exposed to the LLM.
    fn name(&self) -> &str;
    /// Human-readable description for tool selection.
    fn description(&self) -> &str;
    /// JSON schema for accepted tool arguments.
    fn parameters_schema(&self) -> serde_json::Value;
    /// Executes the tool with the given call id and JSON args.
    async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult;
}
