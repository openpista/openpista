//! Agent runtime, memory layer, and LLM adapter interfaces.

pub mod anthropic;
pub mod approval;
pub mod llm;
pub mod memory;
pub mod responses;
pub mod runtime;
pub mod tool_registry;

/// Anthropic Messages API provider.
pub use anthropic::AnthropicProvider;
/// Tool call approval handler trait and auto-approve default.
pub use approval::{AutoApproveHandler, ToolApprovalHandler};
/// Chat request/response models and provider interfaces.
pub use llm::{ChatRequest, ChatResponse, LlmProvider, OpenAiProvider, TokenUsage};
/// SQLite-backed conversation memory implementation.
pub use memory::SqliteMemory;
/// OpenAI Responses API provider (subscription-based billing).
pub use responses::ResponsesApiProvider;
/// Main runtime orchestration loop.
pub use runtime::AgentRuntime;
/// Runtime tool registry.
pub use tool_registry::ToolRegistry;
