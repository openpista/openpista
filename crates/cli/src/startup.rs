//! Provider and runtime construction helpers.

use crate::config::Config;
use proto::SessionId;

#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use crate::auth::is_openai_oauth_credential_for_key;
#[cfg(not(test))]
use crate::config::ProviderPreset;
#[cfg(not(test))]
use agent::{
    AgentRuntime, AnthropicProvider, Memory, OpenAiProvider, ResponsesApiProvider, SqliteMemory,
    ToolRegistry,
};
#[cfg(not(test))]
use tools::{
    BashTool, BrowserClickTool, BrowserScreenshotTool, BrowserTool, BrowserTypeTool, ContainerTool,
    ScreenTool,
};
#[cfg(not(test))]
use tracing::{info, warn};

pub(crate) fn resolve_tui_session_id(
    config: &Config,
    explicit_session: Option<String>,
) -> SessionId {
    if let Some(id) = explicit_session {
        return SessionId::from(id);
    }

    let shared = config.channels.web.shared_session_id.trim();
    if !shared.is_empty() {
        return SessionId::from(shared.to_string());
    }

    SessionId::new()
}

#[cfg(not(test))]
/// Builds an LLM provider instance for the given preset, API key, optional base URL, and model.
pub(crate) fn build_provider(
    preset: ProviderPreset,
    api_key: &str,
    base_url: Option<&str>,
    model: &str,
) -> Arc<dyn agent::LlmProvider> {
    match preset {
        ProviderPreset::Anthropic => {
            if let Some(base_url) = base_url {
                Arc::new(AnthropicProvider::with_base_url(api_key, base_url))
            } else {
                Arc::new(AnthropicProvider::new(api_key))
            }
        }
        _ => {
            // Detect OAuth-based credential → use Responses API for subscription access
            let use_responses_api =
                preset == ProviderPreset::OpenAi && is_openai_oauth_credential_for_key(api_key);
            if use_responses_api {
                let account_id = crate::auth::extract_chatgpt_account_id(api_key);
                let provider = if let Some(base_url) = base_url {
                    ResponsesApiProvider::with_base_url(api_key, base_url)
                } else {
                    ResponsesApiProvider::new(api_key)
                };
                Arc::new(provider.with_chatgpt_account_id(account_id))
            } else if let Some(base_url) = base_url {
                Arc::new(OpenAiProvider::with_base_url(api_key, base_url, model))
            } else {
                Arc::new(OpenAiProvider::new(api_key, model))
            }
        }
    }
}

#[cfg(not(test))]
/// Creates a runtime with configured tools, memory, and LLM provider.
pub(crate) async fn build_runtime(
    config: &Config,
    approval_handler: Arc<dyn proto::ToolApprovalHandler>,
) -> anyhow::Result<Arc<AgentRuntime>> {
    // Tool registry
    let mut registry = ToolRegistry::new();
    registry.register(BashTool::new());
    registry.register(ScreenTool::new());
    registry.register(BrowserTool::new());
    registry.register(BrowserClickTool::new());
    registry.register(BrowserTypeTool::new());
    registry.register(BrowserScreenshotTool::new());
    registry.register(ContainerTool::new());
    let registry = Arc::new(registry);

    // Memory
    let memory = SqliteMemory::open(&config.database.url)
        .await
        .map_err(|e| anyhow::anyhow!("DB error: {e}"))?;
    let memory: Arc<dyn Memory> = Arc::new(memory);

    // LLM provider
    let api_key = config.resolve_api_key_refreshed().await;
    if api_key.is_empty() {
        warn!("No API key configured. Set openpista_API_KEY or OPENAI_API_KEY.");
    }
    let model = config.agent.effective_model().to_string();
    let llm = build_provider(
        config.agent.provider,
        &api_key,
        config.agent.effective_base_url(),
        &model,
    );

    let runtime = Arc::new(AgentRuntime::new(
        llm,
        registry,
        memory,
        config.agent.provider.name(),
        &model,
        config.agent.max_tool_rounds,
        approval_handler,
    ));

    // Register all authenticated providers so the runtime can switch between them.
    for preset in ProviderPreset::all() {
        let name = preset.name();
        // Skip the active provider — already registered by AgentRuntime::new.
        if name == config.agent.provider.name() {
            continue;
        }
        if let Some(cred) = config.resolve_credential_for_refreshed(name).await {
            let default_model = preset.default_model();
            let provider = build_provider(
                *preset,
                &cred.api_key,
                cred.base_url.as_deref(),
                default_model,
            );
            runtime.register_provider(name, provider);
            info!(provider = %name, "Registered additional provider");
        }
    }

    Ok(runtime)
}
