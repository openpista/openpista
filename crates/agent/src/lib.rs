//! Agent runtime, memory layer, and LLM adapter interfaces.

pub mod approval;
pub mod memory;
pub mod provider;
pub mod runtime;
pub mod tool_registry;

/// Tool call approval handler trait and auto-approve default.
pub use approval::{AutoApproveHandler, ToolApprovalHandler};
/// Conversation memory trait (implement to create custom backends).
pub use memory::Memory;
/// SQLite-backed conversation memory implementation.
pub use memory::SqliteMemory;
/// Anthropic Messages API provider.
pub use provider::anthropic::AnthropicProvider;
/// OpenAI-compatible provider.
pub use provider::openai::OpenAiProvider;
/// OpenAI Responses API provider (subscription-based billing).
pub use provider::responses::ResponsesApiProvider;
/// Chat request/response models and provider interfaces.
pub use provider::{ChatRequest, ChatResponse, LlmProvider, TokenUsage};
/// Main runtime orchestration loop.
pub use runtime::AgentRuntime;
/// Runtime tool registry.
pub use tool_registry::ToolRegistry;
