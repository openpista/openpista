//! Async event loop for the TUI — interleaves crossterm, agent progress, and timer events.
#![allow(dead_code, unused_imports)]

use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context;

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use proto::{ChannelId, ProgressEvent, SessionId};
use ratatui::layout::Position;
use ratatui::{Terminal, backend::CrosstermBackend};
use skills::SkillLoader;
use tokio::sync::mpsc;

use super::app::{AppState, TuiApp};
use crate::auth_picker::{AuthLoginIntent, AuthMethodChoice};
use crate::config::{
    Config, LoginAuthMode, OAuthEndpoints, ProviderPreset, provider_registry_entry,
};
use crate::model_catalog;
use tracing::debug;

/// Local port used for the OAuth redirect callback server.
const OAUTH_CALLBACK_PORT: u16 = 9009;

/// Formats model catalog entries into a human-readable text listing for chat display.
fn format_model_list(
    entries: &[model_catalog::ModelCatalogEntry],
    sync_statuses: &[String],
) -> String {
    use model_catalog::ModelSource;
    let recommended: Vec<_> = entries
        .iter()
        .filter(|e| e.recommended_for_coding && e.available)
        .collect();
    let other: Vec<_> = entries
        .iter()
        .filter(|e| !e.recommended_for_coding && e.available)
        .collect();

    let mut out = format!("Models — {} total\n", entries.len());
    if !recommended.is_empty() {
        out.push_str("\nRecommended:\n");
        for e in &recommended {
            let tag = if e.source == ModelSource::Api {
                " (api)"
            } else {
                ""
            };
            out.push_str(&format!("  ★  {} [{}]{}\n", e.id, e.provider, tag));
        }
    }
    if !other.is_empty() {
        out.push_str("\nOther:\n");
        for e in &other {
            let tag = if e.source == ModelSource::Api {
                " (api)"
            } else {
                ""
            };
            out.push_str(&format!("     {} [{}]{}\n", e.id, e.provider, tag));
        }
    }

    if !sync_statuses.is_empty() {
        out.push_str(&format!("\nSync: {}", sync_statuses.join("; ")));
    }
    out
}

/// Collects (provider_name, base_url, api_key) tuples for all authenticated providers.
fn collect_authenticated_providers(config: &Config) -> Vec<(String, Option<String>, String)> {
    use crate::config::ProviderPreset;
    let mut providers = Vec::new();
    for preset in ProviderPreset::all() {
        let name = preset.name();
        if let Some(cred) = config.resolve_credential_for(name) {
            providers.push((name.to_string(), cred.base_url, cred.api_key));
        }
    }
    // Ensure the currently configured provider is always included
    let active = config.agent.provider.name().to_string();
    if !providers.iter().any(|(n, _, _)| n == &active) {
        let key = config.resolve_api_key();
        if !key.is_empty() {
            providers.push((
                active,
                config.agent.effective_base_url().map(String::from),
                key,
            ));
        }
    }
    providers
}
/// Maximum seconds to wait for the OAuth callback before timing out.
const OAUTH_TIMEOUT_SECS: u64 = 120;

/// Parsed sub-command for the `/model` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelsCommand {
    /// Open the interactive model browser.
    Browse,
    /// Print model list to chat.
    List,
    /// Unrecognised sub-command with an error message.
    Invalid(String),
}

/// Parses a raw `/model` input into a `ModelsCommand` variant.
fn parse_models_command(raw: &str) -> Option<ModelsCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/model" {
        return None;
    }

    match parts.next() {
        None => Some(ModelsCommand::Browse),
        Some("list") => Some(ModelsCommand::List),
        Some(_) => Some(ModelsCommand::Invalid(
            "Use /model to browse or /model list to print models.".to_string(),
        )),
    }
}

/// How to display the model catalog once loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelTaskMode {
    /// Open the interactive browser with a search query.
    Browse(String),
    /// Print a text listing to chat.
    List,
}

/// Basic API key validation for interactive TUI login input.
fn validate_api_key(api_key: String) -> Result<String, String> {
    let key = api_key.trim().to_string();
    if key.is_empty() {
        return Err("API key cannot be empty".to_string());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("API key must not contain whitespace".to_string());
    }
    Ok(key)
}

/// Persists one provider credential into credentials storage.
fn persist_credential(
    provider: String,
    credential: crate::auth::ProviderCredential,
    path: std::path::PathBuf,
) -> Result<(), String> {
    let mut creds = load_credentials(&path);
    creds.set(provider, credential);
    creds.save_to(&path).map_err(|e| e.to_string())
}

/// Loads provider credentials from the given TOML file path.
fn load_credentials(path: &std::path::Path) -> crate::auth::Credentials {
    if !path.exists() {
        return crate::auth::Credentials::default();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

/// Returns the default on-disk credentials file path.
fn credentials_path() -> std::path::PathBuf {
    crate::auth::Credentials::path()
}

#[cfg(not(test))]
/// Runs browser OAuth login flow for one provider.
async fn run_oauth_login(
    provider: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
    port: u16,
    timeout: u64,
) -> anyhow::Result<crate::auth::ProviderCredential> {
    crate::auth::login(provider, endpoints, client_id, port, timeout).await
}

#[cfg(test)]
/// Test stub for OAuth flow; keeps tests deterministic.
async fn run_oauth_login(
    _provider: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
    _port: u16,
    _timeout: u64,
) -> anyhow::Result<crate::auth::ProviderCredential> {
    let _ = (
        endpoints.auth_url,
        endpoints.token_url,
        endpoints.scope,
        client_id,
    );
    anyhow::bail!("OAuth login is not available in tests")
}

/// Builds and persists a provider credential using the default credentials path.
pub(crate) async fn build_and_store_credential(
    config: &Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
) -> Result<String, String> {
    build_and_store_credential_with_path(config, intent, port, timeout, credentials_path()).await
}

/// Builds and persists a provider credential to a specified path.
async fn build_and_store_credential_with_path(
    config: &Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
    cred_path: std::path::PathBuf,
) -> Result<String, String> {
    let provider = intent.provider.to_ascii_lowercase();
    let entry = provider_registry_entry(&provider).ok_or_else(|| {
        format!(
            "Unknown provider '{provider}'. Available providers: {}",
            crate::config::provider_registry_names()
        )
    })?;
    let provider_name = entry.name.to_string();
    let resolved_method = intent.auth_method;

    let (credential, success_message) = match entry.auth_mode {
        LoginAuthMode::OAuth => {
            if resolved_method == AuthMethodChoice::ApiKey {
                let raw_key = intent
                    .api_key
                    .ok_or_else(|| "API key input is required".to_string())?;
                let key = validate_api_key(raw_key)?;
                (
                    crate::auth::ProviderCredential {
                        access_token: key,
                        refresh_token: None,
                        expires_at: None,
                        endpoint: intent.endpoint,
                    },
                    format!(
                        "Saved API key for '{provider_name}'. It will be used on the next launch (equivalent to setting {}).",
                        entry.api_key_env
                    ),
                )
            } else {
                let endpoints = ProviderPreset::from_str(entry.name)
                    .ok()
                    .and_then(|p| p.oauth_endpoints())
                    .or_else(|| crate::config::extension_oauth_endpoints(&provider_name))
                    .ok_or_else(|| {
                        format!("Provider '{provider_name}' does not support OAuth login")
                    })?;

                let client_id = endpoints
                    .effective_client_id(&config.agent.oauth_client_id)
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        "No OAuth client ID configured. Set openpista_OAUTH_CLIENT_ID environment variable or add oauth_client_id to [agent] in config.toml.".to_string()
                    })?;

                let oauth_credential = if endpoints.default_callback_port.is_none()
                    && !endpoints.redirect_path.is_empty()
                {
                    let pending = crate::auth::start_code_display_flow(
                        &provider_name,
                        &endpoints,
                        &client_id,
                    );
                    let code = if let Some(c) = intent.api_key.as_deref().filter(|s| !s.is_empty())
                    {
                        c.to_string()
                    } else {
                        crate::auth::read_code_from_stdin()
                            .await
                            .map_err(|e| e.to_string())?
                    };
                    crate::auth::complete_code_display_flow(&pending, &code)
                        .await
                        .map_err(|e| e.to_string())?
                } else {
                    let effective_port = endpoints.default_callback_port.unwrap_or(port);
                    run_oauth_login(
                        &provider_name,
                        &endpoints,
                        &client_id,
                        effective_port,
                        timeout,
                    )
                    .await
                    .map_err(|e| e.to_string())?
                };

                let credential = if provider_name == "anthropic" {
                    match crate::auth::create_anthropic_api_key(&oauth_credential.access_token)
                        .await
                    {
                        Ok(permanent_key) => crate::auth::ProviderCredential {
                            access_token: permanent_key,
                            refresh_token: None,
                            expires_at: None,
                            endpoint: None,
                        },
                        Err(e) => {
                            tracing::warn!(
                                "Failed to create Anthropic API key, using OAuth token directly: {e}"
                            );
                            oauth_credential
                        }
                    }
                } else {
                    oauth_credential
                };

                (
                    credential,
                    format!(
                        "Authenticated as '{provider_name}'. Token stored in {}",
                        cred_path.display()
                    ),
                )
            }
        }
        LoginAuthMode::ApiKey => {
            let raw_key = intent
                .api_key
                .ok_or_else(|| "API key input is required".to_string())?;
            let key = validate_api_key(raw_key)?;
            (
                crate::auth::ProviderCredential {
                    access_token: key,
                    refresh_token: None,
                    expires_at: None,
                    endpoint: intent.endpoint,
                },
                format!(
                    "Saved API key for '{provider_name}'. It will be used on the next launch (equivalent to setting {}).",
                    entry.api_key_env
                ),
            )
        }
        LoginAuthMode::EndpointAndKey => {
            let raw_key = intent
                .api_key
                .ok_or_else(|| "API key input is required".to_string())?;
            let key = validate_api_key(raw_key)?;
            let endpoint = intent
                .endpoint
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Endpoint is required for this provider".to_string())?;

            (
                crate::auth::ProviderCredential {
                    access_token: key,
                    refresh_token: None,
                    expires_at: None,
                    endpoint: Some(endpoint.clone()),
                },
                format!(
                    "Saved endpoint+key for '{provider_name}'. Endpoint stored as {}.",
                    entry.endpoint_env.unwrap_or("PROVIDER_ENDPOINT")
                ),
            )
        }
        LoginAuthMode::None => {
            return Err(format!(
                "Provider '{provider_name}' does not require authentication"
            ));
        }
    };

    tokio::task::spawn_blocking(move || persist_credential(provider_name, credential, cred_path))
        .await
        .map_err(|e| format!("Auth task join failed: {e}"))??;
    if entry.supports_runtime {
        Ok(success_message)
    } else {
        Ok(format!(
            "{} Credential stored; runtime execution not yet wired.",
            success_message
        ))
    }
}

/// Persists authentication data for OAuth/API-key login paths.
async fn persist_auth(
    config: Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
) -> Result<String, String> {
    build_and_store_credential(&config, intent, port, timeout).await
}

/// Test helper that delegates to `build_and_store_credential_with_path`.
#[cfg(test)]
async fn persist_auth_with_path(
    config: Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
    cred_path: std::path::PathBuf,
) -> Result<String, String> {
    build_and_store_credential_with_path(&config, intent, port, timeout, cred_path).await
}

/// RAII guard that restores the terminal on drop (even on panic).
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    }
}

/// Run the full-screen TUI until the user quits.
pub async fn run_tui(
    runtime: Arc<agent::AgentRuntime>,
    skill_loader: Arc<SkillLoader>,
    channel_id: ChannelId,
    session_id: SessionId,
    model_name: String,
    mut config: Config,
) -> anyhow::Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let _guard = TerminalGuard; // Drop restores terminal

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    debug!(session = %session_id, model = %model_name, provider = %config.agent.provider.name(), "TUI started");

    // App state
    let mut app = TuiApp::new(
        &model_name,
        session_id.clone(),
        channel_id.clone(),
        config.agent.provider.name(),
    );

    // Load session list for sidebar
    {
        let memory = runtime.memory().clone();
        if let Ok(sessions) = memory.list_sessions_with_preview().await {
            app.session_list = sessions
                .into_iter()
                .map(
                    |(id, channel_id, updated_at, preview)| super::app::SessionEntry {
                        id,
                        channel_id,
                        updated_at,
                        preview,
                    },
                )
                .collect();
        }
    }

    // Crossterm event stream (async)
    let mut crossterm_stream = EventStream::new();

    // Agent task state
    let mut agent_task: Option<tokio::task::JoinHandle<Result<String, proto::Error>>> = None;
    let mut progress_rx: Option<mpsc::Receiver<ProgressEvent>> = None;
    let mut auth_task: Option<tokio::task::JoinHandle<Result<String, String>>> = None;
    let mut model_task: Option<tokio::task::JoinHandle<model_catalog::MultiCatalogLoadResult>> =
        None;
    let mut model_task_opts: Option<ModelTaskMode> = None;
    let mut pending_code_display: Option<crate::auth::PendingOAuthCodeDisplay> = None;
    let mut auth_provider_name: Option<String> = None;
    let mut prev_provider: Option<(ProviderPreset, String)> = None;

    // Spinner tick interval (100ms)
    let mut spinner_interval = tokio::time::interval(std::time::Duration::from_millis(100));
    spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Render
        terminal.draw(|frame| app.render(frame))?;

        // Event select
        tokio::select! {
            // Branch 1: crossterm terminal events
            maybe_event = crossterm_stream.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        use crossterm::event::KeyCode;
                        if key.code == KeyCode::Enter {
                            // Step 1: palette가 활성화되어 있으면 먼저 command resolve
                            if app.is_palette_active() {
                                app.take_palette_command();
                            }

                            // Step 2: 정상 command 처리 경로로 fall-through
                            if app.state == AppState::Idle && !app.input.is_empty() {
                                let message = app.take_input();
                                if let Some(models_cmd) = parse_models_command(&message) {
                                    if model_task.is_some() {
                                        app.push_error(
                                            "Model sync is already in progress. Please wait."
                                                .to_string(),
                                        );
                                        app.scroll_to_bottom();
                                        continue;
                                    }

                                    match models_cmd {
                                        ModelsCommand::Browse => {
                                            let pname = config.agent.provider.name().to_string();
                                            app.open_model_browser(
                                                pname,
                                                Vec::new(),
                                                String::new(),
                                                "Loading models...".to_string(),
                                            );
                                            model_task_opts = Some(ModelTaskMode::Browse(String::new()));
                                            let providers = collect_authenticated_providers(&config);
                                            model_task = Some(tokio::spawn(async move {
                                                model_catalog::load_catalog_multi(&providers).await
                                            }));
                                        }
                                        ModelsCommand::List => {
                                            app.push_assistant("Fetching model list…".to_string());
                                            model_task_opts = Some(ModelTaskMode::List);
                                            let providers = collect_authenticated_providers(&config);
                                            model_task = Some(tokio::spawn(async move {
                                                model_catalog::load_catalog_multi(&providers).await
                                            }));
                                        }
                                        ModelsCommand::Invalid(message) => {
                                            app.push_error(message);
                                        }
                                    }
                                    app.scroll_to_bottom();
                                    continue;
                                }

                                if app.handle_slash_command(&message) {
                                    debug!(command = %message, "Slash command dispatched");
                                    app.scroll_to_bottom();
                                    continue;
                                }
                                debug!(message_len = %message.len(), "Agent task spawned");
                                app.push_user(message.clone());
                                app.state = AppState::Thinking { round: 0 };
                                app.scroll_to_bottom();

                                // Spawn agent task
                                let (prog_tx, prog_rx_new) = mpsc::channel::<ProgressEvent>(64);
                                let rt = Arc::clone(&runtime);
                                let sl = Arc::clone(&skill_loader);
                                let ch = channel_id.clone();
                                let sess = session_id.clone();

                                let handle = tokio::spawn(async move {
                                    let skills_ctx = sl.load_context().await;
                                    rt.process_with_progress(
                                        &ch,
                                        &sess,
                                        &message,
                                        Some(&skills_ctx),
                                        prog_tx,
                                    )
                                    .await
                                });

                                agent_task = Some(handle);
                                progress_rx = Some(prog_rx_new);
                            } else {
                                app.handle_key(key);
                            }
                        } else {
                            app.handle_key(key);
                        }

                        if let Some((new_model, provider_name)) = app.take_pending_model_change() {
                            runtime.set_model(new_model.clone());
                            if provider_name != runtime.active_provider_name() {
                                // Try switching first; if provider not registered, build & register on-demand
                                if runtime.switch_provider(&provider_name).is_err() {
                                    if let Ok(preset) = provider_name.parse::<ProviderPreset>() {
                                        if let Some(cred) = config.resolve_credential_for(&provider_name) {
                                            let new_llm: Arc<dyn agent::LlmProvider> =
                                                if preset == ProviderPreset::Anthropic {
                                                    if let Some(ref url) = cred.base_url {
                                                        Arc::new(agent::AnthropicProvider::with_base_url(&cred.api_key, url))
                                                    } else {
                                                        Arc::new(agent::AnthropicProvider::new(&cred.api_key))
                                                    }
                                                } else if let Some(ref url) = cred.base_url {
                                                    Arc::new(agent::OpenAiProvider::with_base_url(&cred.api_key, url, &new_model))
                                                } else {
                                                    Arc::new(agent::OpenAiProvider::new(&cred.api_key, &new_model))
                                                };
                                            runtime.register_provider(&provider_name, new_llm);
                                            let _ = runtime.switch_provider(&provider_name);
                                        } else {
                                            tracing::warn!(provider = %provider_name, "No credential found for provider");
                                        }
                                    } else {
                                        tracing::warn!(provider = %provider_name, "Unknown provider preset");
                                    }
                                }
                            }
                        }

                        if app.take_model_refresh_request() {
                            if model_task.is_some() {
                                app.push_error(
                                    "Model sync is already in progress. Please wait."
                                        .to_string(),
                                );
                            } else if let Some(query) = app.model_browser_query() {
                                app.mark_model_refreshing();
                                model_task_opts = Some(ModelTaskMode::Browse(query));
                                let providers = collect_authenticated_providers(&config);
                                model_task = Some(tokio::spawn(async move {
                                    model_catalog::load_catalog_multi(&providers).await
                                }));
                            }
                        }

                        if auth_task.is_none()
                            && let Some(intent) = app.take_pending_auth_intent()
                        {
                            if intent.auth_method == AuthMethodChoice::OAuth
                                && !crate::config::oauth_available_for(
                                    &intent.provider,
                                    &config.agent.oauth_client_id,
                                )
                            {
                                if intent.provider == "openai"
                                    || intent.provider == "anthropic"
                                {
                                    app.reopen_method_selector_with_error(
                                        &intent.provider,
                                        "No OAuth client ID configured. Choose API key mode or set openpista_OAUTH_CLIENT_ID.".to_string(),
                                    );
                                } else {
                                    app.reopen_provider_selection_with_error(
                                        "No OAuth client ID configured. Set openpista_OAUTH_CLIENT_ID to use browser login.".to_string(),
                                    );
                                }
                                app.scroll_to_bottom();
                                continue;
                            }

                            // Code-display OAuth phase 1: open browser, prompt for code
                            if intent.auth_method == AuthMethodChoice::OAuth
                                && intent.api_key.is_none()
                            {
                                let ep = std::str::FromStr::from_str(&intent.provider)
                                    .ok()
                                    .and_then(|p: ProviderPreset| p.oauth_endpoints())
                                    .or_else(|| {
                                        crate::config::extension_oauth_endpoints(&intent.provider)
                                    });

                                if let Some(ref ep) = ep
                                    && ep.default_callback_port.is_none()
                                    && !ep.redirect_path.is_empty()
                                {
                                    let client_id = ep
                                        .effective_client_id(&config.agent.oauth_client_id)
                                        .unwrap_or_default()
                                        .to_string();
                                    let pending = crate::auth::start_code_display_flow(
                                        &intent.provider,
                                        ep,
                                        &client_id,
                                    );
                                    pending_code_display = Some(pending);
                                    app.state = super::app::AppState::LoginBrowsing {
                                        query: intent.provider.clone(),
                                        cursor: 0,
                                        scroll: 0,
                                        step: crate::auth_picker::LoginBrowseStep::InputApiKey,
                                        selected_provider: Some(intent.provider),
                                        selected_method: Some(AuthMethodChoice::OAuth),
                                        input_buffer: String::new(),
                                        masked_buffer: String::new(),
                                        last_error: None,
                                        endpoint: None,
                                    };
                                    app.push_assistant(
                                        "Browser opened. Paste the authorization code from your browser.".to_string(),
                                    );
                                    app.scroll_to_bottom();
                                    continue;
                                }
                            }

                            // Code-display OAuth phase 2: exchange code for token
                            if intent.auth_method == AuthMethodChoice::OAuth
                                && intent.api_key.is_some()
                                && pending_code_display.is_some()
                            {
                                let pending = pending_code_display.take().unwrap();
                                let code = intent.api_key.clone().unwrap();
                                let provider_name = intent.provider.clone();
                                let cred_path = credentials_path();
                                auth_provider_name = Some(provider_name.clone());
                                app.state = super::app::AppState::AuthValidating {
                                    provider: provider_name.clone(),
                                };
                                if let Ok(preset) = provider_name.parse::<ProviderPreset>() {
                                    prev_provider = Some((config.agent.provider, app.provider_name.clone()));
                                    config.agent.provider = preset;
                                    app.provider_name = preset.name().to_string();
                                }
                                auth_task = Some(tokio::spawn(async move {
                                    let oauth_cred =
                                        crate::auth::complete_code_display_flow(&pending, &code)
                                            .await
                                            .map_err(|e| e.to_string())?;
                                    let credential = if provider_name == "anthropic" {
                                        match crate::auth::create_anthropic_api_key(
                                            &oauth_cred.access_token,
                                        )
                                        .await
                                        {
                                            Ok(api_key) => crate::auth::ProviderCredential {
                                                access_token: api_key,
                                                refresh_token: None,
                                                expires_at: None,
                                                endpoint: None,
                                            },
                                            Err(e) => {
                                                tracing::warn!(
                                                    "Failed to create Anthropic API key, using OAuth token directly: {e}"
                                                );
                                                oauth_cred
                                            }
                                        }
                                    } else {
                                        oauth_cred
                                    };
                                    let p = provider_name.clone();
                                    tokio::task::spawn_blocking(move || {
                                        persist_credential(p, credential, cred_path)
                                    })
                                    .await
                                    .map_err(|e| format!("Join failed: {e}"))??;
                                    Ok(format!(
                                        "Authenticated as '{provider_name}'. API key saved."
                                    ))
                                }));
                                app.scroll_to_bottom();
                                continue;
                            }

                            auth_provider_name = Some(intent.provider.clone());
                            if let Ok(preset) = intent.provider.parse::<ProviderPreset>() {
                                prev_provider = Some((config.agent.provider, app.provider_name.clone()));
                                config.agent.provider = preset;
                                app.provider_name = preset.name().to_string();
                            }
                            let config_for_task = config.clone();
                            auth_task = Some(tokio::spawn(async move {
                                persist_auth(
                                    config_for_task,
                                    intent,
                                    OAUTH_CALLBACK_PORT,
                                    OAUTH_TIMEOUT_SECS,
                                )
                                .await
                            }));
                        }
                    }
                    Some(Ok(Event::Mouse(mouse))) => {
                        let frame_area: ratatui::layout::Rect = terminal.size().unwrap_or_default().into();
                        let pos = Position::new(mouse.column, mouse.row);

                        // ── Sidebar mouse handling ───────────────────────
                        if let Some(sb_area) = app.compute_sidebar_area(frame_area) {
                            match mouse.kind {
                                MouseEventKind::Down(MouseButton::Left) => {
                                    if sb_area.contains(pos) {
                                        let inner_y = mouse.row.saturating_sub(sb_area.y + 1);
                                        let entry_height = 3u16;
                                        let idx = (inner_y / entry_height) as usize;
                                        if idx < app.session_list.len() {
                                            app.sidebar_hover = Some(idx);
                                        }
                                    }
                                }
                                MouseEventKind::Moved => {
                                    if sb_area.contains(pos) {
                                        let inner_y = mouse.row.saturating_sub(sb_area.y + 1);
                                        let entry_height = 3u16;
                                        let idx = (inner_y / entry_height) as usize;
                                        if idx < app.session_list.len() {
                                            app.sidebar_hover = Some(idx);
                                        } else {
                                            app.sidebar_hover = None;
                                        }
                                    } else {
                                        app.sidebar_hover = None;
                                    }
                                }
                                MouseEventKind::ScrollDown => {
                                    if sb_area.contains(pos) {
                                        app.sidebar_scroll = app.sidebar_scroll.saturating_add(1);
                                    }
                                }
                                MouseEventKind::ScrollUp => {
                                    if sb_area.contains(pos) {
                                        app.sidebar_scroll = app.sidebar_scroll.saturating_sub(1);
                                    }
                                }
                                _ => {}
                            }
                        }

                        // ── Chat area mouse handling ──────────────────────
                        if let Some(chat_area) = app.chat_area {
                            let inner = ratatui::layout::Rect {
                                x: chat_area.x + 1,
                                y: chat_area.y + 1,
                                width: chat_area.width.saturating_sub(2),
                                height: chat_area.height.saturating_sub(2),
                            };

                            match mouse.kind {
                                MouseEventKind::Down(MouseButton::Left) => {
                                    if inner.contains(pos) {
                                        let rel_col = mouse.column - inner.x;
                                        let rel_row = mouse.row - inner.y;
                                        app.text_selection.anchor = Some((rel_row, rel_col));
                                        app.text_selection.endpoint = Some((rel_row, rel_col));
                                        app.text_selection.dragging = true;
                                    } else {
                                        app.text_selection.clear();
                                    }
                                }
                                MouseEventKind::Drag(MouseButton::Left) => {
                                    if app.text_selection.dragging {
                                        let rel_col = mouse
                                            .column
                                            .saturating_sub(inner.x)
                                            .min(inner.width.saturating_sub(1));
                                        let rel_row = mouse
                                            .row
                                            .saturating_sub(inner.y)
                                            .min(inner.height.saturating_sub(1));
                                        app.text_selection.endpoint = Some((rel_row, rel_col));
                                    }
                                }
                                MouseEventKind::Up(MouseButton::Left) => {
                                    if app.text_selection.dragging {
                                        let rel_col = mouse
                                            .column
                                            .saturating_sub(inner.x)
                                            .min(inner.width.saturating_sub(1));
                                        let rel_row = mouse
                                            .row
                                            .saturating_sub(inner.y)
                                            .min(inner.height.saturating_sub(1));
                                        app.text_selection.endpoint = Some((rel_row, rel_col));
                                        app.text_selection.dragging = false;

                                        // Auto-copy when a non-empty selection is released.
                                        if app.text_selection.is_active()
                                            && let Some((start, end)) =
                                                app.text_selection.ordered_range()
                                        {
                                            let grid = app.chat_text_grid.clone();
                                            let scroll = app.chat_scroll_clamped;
                                            if let Some(text) =
                                                crate::tui::selection::extract_selected_text(
                                                    &grid, start, end, scroll,
                                                )
                                            {
                                                crate::tui::selection::copy_to_clipboard(&text);
                                            }
                                        }
                                    }
                                }
                                MouseEventKind::ScrollDown => {
                                    if chat_area.contains(pos) {
                                        app.history_scroll =
                                            app.history_scroll.saturating_add(3);
                                        app.text_selection.clear();
                                    }
                                }
                                MouseEventKind::ScrollUp => {
                                    if chat_area.contains(pos) {
                                        app.history_scroll =
                                            app.history_scroll.saturating_sub(3);
                                        app.text_selection.clear();
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => {
                        // Terminal will redraw on next loop iteration
                    }
                    Some(Err(_)) | None => {
                        break; // stream ended or error
                    }
                    _ => {}
                }
            }

            // Branch 2: progress events from agent task
            Some(evt) = async {
                match progress_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                app.apply_progress(evt);
                app.scroll_to_bottom();
            }

            // Branch 3: agent task completed
            result = async {
                match agent_task.as_mut() {
                    Some(handle) => handle.await,
                    None => std::future::pending().await,
                }
            } => {
                match result {
                    Ok(inner) => {
                        debug!(success = %inner.is_ok(), "Agent task completed");
                        app.apply_completion(inner);
                    }
                    Err(join_err) => app.apply_completion(Err(proto::Error::Llm(
                        proto::LlmError::InvalidResponse(format!("Task panicked: {join_err}"))
                    ))),
                }
                app.scroll_to_bottom();
                agent_task = None;
                progress_rx = None;
            }

            result = async {
                match auth_task.as_mut() {
                    Some(handle) => handle.await,
                    None => std::future::pending().await,
                }
            } => {
                match result {
                    Ok(Ok(message)) => {
                        if let Some(ref provider_str) = auth_provider_name
                            && let Ok(preset) = provider_str.parse::<ProviderPreset>()
                        {
                            let new_model = preset.default_model().to_string();
                            runtime.set_model(new_model.clone());
                            let api_key = config.resolve_api_key();
                            let new_llm: Arc<dyn agent::LlmProvider> =
                                if preset == ProviderPreset::Anthropic {
                                    if let Some(base_url) = config.agent.effective_base_url() {
                                        Arc::new(agent::AnthropicProvider::with_base_url(&api_key, base_url))
                                    } else {
                                        Arc::new(agent::AnthropicProvider::new(&api_key))
                                    }
                                } else {
                                    let burl = config.agent.effective_base_url().map(String::from);
                                    if let Some(url) = burl {
                                        Arc::new(agent::OpenAiProvider::with_base_url(&api_key, url, &new_model))
                                    } else {
                                        Arc::new(agent::OpenAiProvider::new(&api_key, &new_model))
                                    }
                                };
                            runtime.register_provider(provider_str, new_llm);
                            let _ = runtime.switch_provider(provider_str);
                        }
                        debug!(provider = ?auth_provider_name, "Auth task completed successfully");
                        // Pre-cache model catalog for the newly authenticated provider
                        if model_task.is_none() {
                            let providers = collect_authenticated_providers(&config);
                            debug!("Pre-caching model catalog after auth for {} provider(s)", providers.len());
                            model_task = Some(tokio::spawn(async move {
                                model_catalog::load_catalog_multi(&providers).await
                            }));
                        }
                        prev_provider = None;
                        auth_provider_name = None;
                        app.push_assistant(message);
                    }
                    Ok(Err(err)) => {
                        debug!(provider = ?auth_provider_name, error = %err, "Auth task failed");
                        if let Some((old_preset, old_name)) = prev_provider.take() {
                            config.agent.provider = old_preset;
                            app.provider_name = old_name;
                        }
                        auth_provider_name = None;
                        app.push_error(format!("Authentication failed: {err}"));
                    }
                    Err(join_err) => {
                        debug!(provider = ?auth_provider_name, error = %join_err, "Auth task panicked");
                        if let Some((old_preset, old_name)) = prev_provider.take() {
                            config.agent.provider = old_preset;
                            app.provider_name = old_name;
                        }
                        auth_provider_name = None;
                        app.push_error(format!("Auth task failed: {join_err}"));
                    }
                }
                app.state = AppState::Idle;
                app.scroll_to_bottom();
                auth_task = None;
            }

            result = async {
                match model_task.as_mut() {
                    Some(handle) => handle.await,
                    None => std::future::pending().await,
                }
            } => {
                match result {
                    Ok(catalog) => {
                        debug!(entries = %catalog.entries.len(), "Model catalog loaded");
                        let provider_label = if catalog.sync_statuses.len() == 1 {
                            catalog.sync_statuses[0].split(':').next().unwrap_or("unknown").to_string()
                        } else {
                            "multi".to_string()
                        };
                        let sync_status_combined = catalog.sync_statuses.join(" | ");
                        match model_task_opts.take() {
                            Some(ModelTaskMode::Browse(query)) => {
                                app.open_model_browser(
                                    provider_label,
                                    catalog.entries,
                                    query,
                                    sync_status_combined,
                                );
                            }
                            Some(ModelTaskMode::List) => {
                                let text = format_model_list(&catalog.entries, &catalog.sync_statuses);
                                app.push_assistant(text);
                            }
                            None => {
                                // background pre-cache only — no browser opened
                            }
                        }
                    }
                    Err(join_err) => {
                        debug!(error = %join_err, "Model task failed");
                        app.push_error(format!("Model task failed: {join_err}"));
                    }
                }
                model_task = None;
                app.scroll_to_bottom();
            }

            _ = spinner_interval.tick(), if app.state != AppState::Idle => {
                app.spinner_tick = app.spinner_tick.wrapping_add(1);
            }
        }

        if app.should_quit {
            break;
        }
    }

    // TerminalGuard::drop handles cleanup
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_guard_drop_path_is_safe() {
        let guard = TerminalGuard;
        drop(guard);
    }

    #[test]
    fn parse_models_command_supports_browse_variants() {
        assert_eq!(parse_models_command("/model"), Some(ModelsCommand::Browse));
        assert_eq!(
            parse_models_command("/model list"),
            Some(ModelsCommand::List)
        );
        assert_eq!(
            parse_models_command("/model all"),
            Some(ModelsCommand::Invalid(
                "Use /model to browse or /model list to print models.".to_string()
            ))
        );
        assert_eq!(
            parse_models_command("/model refresh"),
            Some(ModelsCommand::Invalid(
                "Use /model to browse or /model list to print models.".to_string()
            ))
        );
        assert_eq!(
            parse_models_command("/model search codex"),
            Some(ModelsCommand::Invalid(
                "Use /model to browse or /model list to print models.".to_string()
            ))
        );
    }

    #[test]
    fn validate_api_key_rejects_empty_and_whitespace() {
        assert!(validate_api_key("".to_string()).is_err());
        assert!(validate_api_key("   ".to_string()).is_err());
        assert!(validate_api_key("abc def".to_string()).is_err());
        assert_eq!(validate_api_key("sk-test".to_string()).unwrap(), "sk-test");
    }

    #[tokio::test]
    async fn persist_auth_api_key_saves_credential() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");

        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "together".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: Some("tok-together".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path.clone(),
        )
        .await;

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&cred_path).expect("read credentials");
        let creds: crate::auth::Credentials = toml::from_str(&content).expect("parse credentials");
        let saved = creds.get("together").expect("credential saved");
        assert_eq!(saved.access_token, "tok-together");
        assert_eq!(saved.endpoint, None);
    }

    #[tokio::test]
    async fn persist_auth_endpoint_and_key_saves_endpoint() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");

        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "custom".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: Some("https://example.azure.com".to_string()),
                api_key: Some("tok-custom".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path.clone(),
        )
        .await;

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&cred_path).expect("read credentials");
        let creds: crate::auth::Credentials = toml::from_str(&content).expect("parse credentials");
        let saved = creds.get("custom").expect("credential saved");
        assert_eq!(saved.access_token, "tok-custom");
        assert_eq!(saved.endpoint.as_deref(), Some("https://example.azure.com"));
    }

    #[tokio::test]
    async fn persist_auth_anthropic_api_key_saves_credential() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");

        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "anthropic".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: Some("sk-ant-test123".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path.clone(),
        )
        .await;

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&cred_path).expect("read credentials");
        let creds: crate::auth::Credentials = toml::from_str(&content).expect("parse credentials");
        let saved = creds.get("anthropic").expect("credential saved");
        assert_eq!(saved.access_token, "sk-ant-test123");
    }

    #[tokio::test]
    async fn persist_auth_anthropic_oauth_fails_in_test_mode() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");

        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "anthropic".to_string(),
                auth_method: AuthMethodChoice::OAuth,
                endpoint: None,
                api_key: None,
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path.clone(),
        )
        .await;

        assert!(result.is_err());
    }
}
