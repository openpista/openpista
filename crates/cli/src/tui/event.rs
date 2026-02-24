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
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;

use super::app::{AppState, TuiApp};
use crate::auth_picker::{AuthLoginIntent, AuthMethodChoice};
use crate::config::{
    Config, LoginAuthMode, OAuthEndpoints, ProviderPreset, provider_registry_entry,
};
use crate::model_catalog;
use tracing::{debug, info};

/// Detects the local IP address by connecting a UDP socket to a public DNS server.
fn detect_local_ip() -> String {
    use std::net::UdpSocket;
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "localhost".to_string())
}

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
const MODEL_SYNC_IN_PROGRESS_MESSAGE: &str = "Model sync is already in progress. Please wait.";

fn model_sync_in_progress_error() -> String {
    MODEL_SYNC_IN_PROGRESS_MESSAGE.to_string()
}

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

/// Parsed sub-command for the `/session` slash command.
#[derive(Debug, Clone, PartialEq)]
enum SessionCommand {
    /// `/session` or `/session list` — print all sessions to chat.
    List,
    /// `/session new` — create a new session.
    New,
    /// `/session load <id>` — load a specific session (partial ID match).
    Load(String),
    /// `/session delete <id>` — delete a specific session (partial ID match).
    Delete(String),
    /// Invalid usage with hint message.
    Invalid(String),
}

fn parse_session_command(raw: &str) -> Option<SessionCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/session" {
        return None;
    }
    match parts.next() {
        None | Some("list") => Some(SessionCommand::List),
        Some("new") => Some(SessionCommand::New),
        Some("load") => {
            let id = parts.collect::<Vec<_>>().join(" ");
            if id.is_empty() {
                Some(SessionCommand::Invalid(
                    "Usage: /session load <id>".to_string(),
                ))
            } else {
                Some(SessionCommand::Load(id))
            }
        }
        Some("delete") | Some("del") => {
            let id = parts.collect::<Vec<_>>().join(" ");
            if id.is_empty() {
                Some(SessionCommand::Invalid(
                    "Usage: /session delete <id>".to_string(),
                ))
            } else {
                Some(SessionCommand::Delete(id))
            }
        }
        Some(_) => Some(SessionCommand::Invalid(
            "Use /session, /session list, /session new, /session load <id>, /session delete <id>"
                .to_string(),
        )),
    }
}

/// Parsed sub-command for the `/web` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WebCommand {
    /// `/web` or `/web status` — display current web config.
    Status,
    /// `/web setup` — launch interactive configuration wizard.
    Setup,
    /// Unrecognised sub-command.
    Invalid(String),
}

fn parse_web_command(raw: &str) -> Option<WebCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/web" {
        return None;
    }
    match parts.next() {
        None | Some("status") => Some(WebCommand::Status),
        Some("setup") => Some(WebCommand::Setup),
        Some(_) => Some(WebCommand::Invalid(
            "Use /web to show status or /web setup to configure.".to_string(),
        )),
    }
}

/// Parsed sub-command for the `/whatsapp` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WhatsAppCommand {
    /// `/whatsapp` — open the interactive setup wizard.
    Setup,
    /// `/whatsapp status` — show current WhatsApp configuration.
    Status,
    /// Invalid sub-command.
    Invalid(String),
}

fn parse_whatsapp_command(raw: &str) -> Option<WhatsAppCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/whatsapp" {
        return None;
    }
    match parts.next() {
        None | Some("setup") => Some(WhatsAppCommand::Setup),
        Some("status") => Some(WhatsAppCommand::Status),
        Some(_) => Some(WhatsAppCommand::Invalid(
            "Usage: /whatsapp [setup|status]".to_string(),
        )),
    }
}

/// Render a QR code as Unicode half-block text lines.
/// Uses `▀`, `▄`, `█` and space to pack two module rows per text line.
pub(crate) fn render_qr_text(url: &str) -> Option<String> {
    use qrcode::QrCode;
    let code = QrCode::new(url.as_bytes()).ok()?;
    let modules = code.to_colors();
    let width = code.width();
    let height = modules.len() / width;

    // Add 1-module quiet zone on each side
    let mut lines: Vec<String> = Vec::new();

    // Top quiet-zone row (all white)
    lines.push(" ".repeat(width + 2));

    // Process two rows at a time using half-block characters
    let mut y = 0;
    while y < height {
        let mut row = String::new();
        row.push(' '); // left quiet zone
        for x in 0..width {
            let top = modules[y * width + x];
            let bottom = if y + 1 < height {
                modules[(y + 1) * width + x]
            } else {
                qrcode::Color::Light // pad with white if odd height
            };
            match (top, bottom) {
                (qrcode::Color::Dark, qrcode::Color::Dark) => row.push('\u{2588}'),
                (qrcode::Color::Dark, qrcode::Color::Light) => row.push('\u{2580}'),
                (qrcode::Color::Light, qrcode::Color::Dark) => row.push('\u{2584}'),
                (qrcode::Color::Light, qrcode::Color::Light) => row.push(' '),
            }
        }
        row.push(' '); // right quiet zone
        lines.push(row);
        y += 2;
    }

    // Bottom quiet-zone row
    lines.push(" ".repeat(width + 2));

    Some(lines.join("\n"))
}
fn format_whatsapp_status(config: &Config) -> String {
    let wa = &config.channels.whatsapp;
    let mut lines = vec!["WhatsApp Configuration Status".to_string(), "".to_string()];
    lines.push(format!(
        "  Enabled:         {}",
        if wa.enabled { "Yes" } else { "No" }
    ));
    lines.push(format!("  Session Dir:     {}", wa.session_dir));
    lines.push(format!(
        "  Bridge Path:     {}",
        wa.bridge_path.as_deref().unwrap_or("(bundled default)")
    ));
    lines.push("".to_string());
    if wa.is_configured() {
        lines.push("  Status: Ready (session directory set)".to_string());
    } else {
        lines.push("  Status: Incomplete \u{2014} run /whatsapp to configure".to_string());
    }
    lines.join("\n")
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

/// Applies provider-specific post-OAuth token exchange.
///
/// GitHub Copilot requires exchanging the GitHub OAuth token for a
/// Copilot-specific session token.  For all other providers the credential
/// is returned unchanged.
async fn maybe_exchange_copilot_token(
    provider_name: &str,
    credential: crate::auth::ProviderCredential,
) -> Result<crate::auth::ProviderCredential, String> {
    if provider_name == "github-copilot" {
        #[cfg(not(test))]
        {
            return crate::auth::exchange_github_copilot_token(&credential.access_token)
                .await
                .map_err(|e| format!("Copilot token exchange failed: {e}"));
        }
        #[cfg(test)]
        {
            Ok(credential)
        }
    } else {
        Ok(credential)
    }
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
                        id_token: None,
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

                let credential =
                    maybe_exchange_copilot_token(&provider_name, oauth_credential).await?;

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
                    id_token: None,
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
                    id_token: None,
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
    mut session_id: SessionId,
    model_name: String,
    mut config: Config,
    mut approval_rx: mpsc::Receiver<super::approval::PendingApproval>,
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

    // Resume existing session: load messages if the session already has history
    {
        let memory = runtime.memory().clone();
        if let Ok(messages) = memory.load_session(&session_id).await
            && !messages.is_empty()
        {
            app.load_session_messages(session_id.clone(), messages);
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

    // WhatsApp bridge subprocess state
    let mut whatsapp_bridge_child: Option<tokio::process::Child> = None;
    let mut whatsapp_qr_rx: Option<mpsc::Receiver<String>> = None;
    let mut whatsapp_connected_rx: Option<mpsc::Receiver<(String, String)>> = None;

    // Spinner tick interval (100ms)
    let mut spinner_interval = tokio::time::interval(std::time::Duration::from_millis(100));
    spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        use super::action::{Action, Command};
        // Render
        terminal.draw(|frame| app.render(frame))?;

        // Event select
        tokio::select! {
            // ── Branch 1: crossterm terminal events ──────────────
            maybe_event = crossterm_stream.next() => {


                // Helper closure: execute a Command produced by update().
                // Synchronous commands are handled inline; async commands are
                // returned so the caller can process them with async context.
                let execute_command = |cmd: Command| -> Command {
                    match cmd {
                        Command::None => Command::None,
                        Command::CopyToClipboard(text) => {
                            crate::tui::selection::copy_to_clipboard(&text);
                            Command::None
                        }
                        Command::Batch(cmds) => {
                            let mut pending = Vec::new();
                            for c in cmds {
                                match c {
                                    Command::None => {}
                                    Command::CopyToClipboard(text) => {
                                        crate::tui::selection::copy_to_clipboard(&text);
                                    }
                                    other => pending.push(other),
                                }
                            }
                            match pending.len() {
                                0 => Command::None,
                                1 => pending.into_iter().next().unwrap(),
                                _ => Command::Batch(pending),
                            }
                        }
                        // Async commands are returned as-is for the event loop to handle.
                        other => other,
                    }
                };

                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        use crossterm::event::KeyCode;
                        let mut pending_async_cmd = Command::None;

                        // Handle tool approval prompt keys first
                        if app.pending_approval.is_some() {
                            match key.code {
                                KeyCode::Char('y') | KeyCode::Char('Y') => {
                                    if let Some(pending) = app.pending_approval.take() {
                                        let _ = pending.reply_tx.send(proto::ToolApprovalDecision::Approve);
                                    }
                                    continue;
                                }
                                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                    if let Some(pending) = app.pending_approval.take() {
                                        let _ = pending.reply_tx.send(proto::ToolApprovalDecision::Reject);
                                    }
                                    continue;
                                }
                                KeyCode::Char('a') | KeyCode::Char('A') => {
                                    if let Some(pending) = app.pending_approval.take() {
                                        let _ = pending.reply_tx.send(proto::ToolApprovalDecision::AllowForSession);
                                    }
                                    continue;
                                }
                                _ => continue, // Ignore other keys while approval is pending
                            }
                        }
                        if key.code == KeyCode::Enter {
                            // Step 1: resolve palette if active
                            if app.is_palette_active() {
                                let cmd = app.update(Action::PaletteSelect);
                                let returned = execute_command(cmd);
                                if !matches!(returned, Command::None) {
                                    pending_async_cmd = returned;
                                }
                            }

                            // Step 2: process Enter in idle state with input
                            if app.state == AppState::Idle && !app.input.is_empty() {
                                let message = app.take_input();
                                if let Some(models_cmd) = parse_models_command(&message) {
                                    if model_task.is_some() {
                                        app.update(Action::PushError(
                                            "Model sync is already in progress. Please wait."
                                                .to_string(),
                                        ));
                                        app.update(Action::ScrollToBottom);
                                        continue;
                                    }

                                    match models_cmd {
                                        ModelsCommand::Browse => {
                                            let pname = config.agent.provider.name().to_string();
                                            app.update(Action::OpenModelBrowser {
                                                provider: pname,
                                                entries: Vec::new(),
                                                query: String::new(),
                                                sync_status: "Loading models...".to_string(),
                                            });
                                            model_task_opts = Some(ModelTaskMode::Browse(String::new()));
                                            let providers = collect_authenticated_providers(&config);
                                            model_task = Some(tokio::spawn(async move {
                                                model_catalog::load_catalog_multi(&providers).await
                                            }));
                                        }
                                        ModelsCommand::List => {
                                            app.update(Action::PushAssistantMessage("Fetching model list…".to_string()));
                                            model_task_opts = Some(ModelTaskMode::List);
                                            let providers = collect_authenticated_providers(&config);
                                            model_task = Some(tokio::spawn(async move {
                                                model_catalog::load_catalog_multi(&providers).await
                                            }));
                                        }
                                        ModelsCommand::Invalid(message) => {
                                            app.update(Action::PushError(message));
                                        }
                                    }
                                    app.update(Action::ScrollToBottom);
                                    continue;
                                }

                                if let Some(session_cmd) = parse_session_command(&message) {
                                    match session_cmd {
                                        SessionCommand::List => {
                                            app.update(Action::OpenSessionBrowser);
                                        }
                                        SessionCommand::New => {
                                            let new_sid = proto::SessionId::new();
                                            app.update(Action::NewSession(new_sid.clone()));
                                            session_id = new_sid;
                                        }
                                        SessionCommand::Load(partial_id) => {
                                            let matched: Vec<_> = app.session_list.iter()
                                                .filter(|e| e.id.as_str().contains(&partial_id))
                                                .collect();
                                            match matched.len() {
                                                0 => { app.update(Action::PushError(format!("No session matching '{partial_id}'"))); }
                                                1 => {
                                                    let sid = matched[0].id.clone();
                                                    app.set_pending_sidebar_selection(sid);
                                                }
                                                n => {
                                                    let ids: Vec<_> = matched.iter().map(|e| format!("`{}`", e.id.as_str())).collect();
                                                    app.update(Action::PushError(format!("{n} sessions match '{partial_id}': {}", ids.join(", "))));
                                                }
                                            }
                                        }
                                        SessionCommand::Delete(partial_id) => {
                                            let matched: Vec<_> = app.session_list.iter()
                                                .filter(|e| e.id.as_str().contains(&partial_id))
                                                .collect();
                                            match matched.len() {
                                                0 => { app.update(Action::PushError(format!("No session matching '{partial_id}'"))); }
                                                1 => {
                                                    let idx = app.session_list.iter().position(|e| e.id.as_str() == matched[0].id.as_str());
                                                    if let Some(i) = idx {
                                                        app.update(Action::SidebarHover(Some(i)));
                                                        app.update(Action::RequestDeleteSession);
                                                    }
                                                }
                                                n => {
                                                    let ids: Vec<_> = matched.iter().map(|e| format!("`{}`", e.id.as_str())).collect();
                                                    app.update(Action::PushError(format!("{n} sessions match '{partial_id}': {}", ids.join(", "))));
                                                }
                                            }
                                        }
                                        SessionCommand::Invalid(msg) => {
                                            app.update(Action::PushError(msg));
                                        }
                                    }
                                    app.update(Action::ScrollToBottom);
                                    continue;
                                }

                                if message.trim() == "/qr" {
                                    if config.channels.web.enabled {
                                        let ip = detect_local_ip();
                                        let port = config.channels.web.port;
                                        let url = format!("http://{ip}:{port}");
                                        match crate::tui::app::generate_qr_lines(&url) {
                                            Ok(qr_lines) => {
                                                app.update(Action::OpenQrCode { url, qr_lines });
                                            }
                                            Err(e) => {
                                                app.update(Action::PushError(format!(
                                                    "Failed to generate QR code: {e}"
                                                )));
                                            }
                                        }
                                    } else {
                                        app.update(Action::PushError(
                                            "Web adapter is not enabled. Set [channels.web] enabled = true in config.toml"
                                                .to_string(),
                                        ));
                                    }
                                    app.update(Action::ScrollToBottom);
                                    continue;
                                }


                                if let Some(web_cmd) = parse_web_command(&message) {
                                    match web_cmd {
                                        WebCommand::Status => {
                                            let wc = &config.channels.web;
                                            let token_set = if wc.token.is_empty() { "no" } else { "yes" };
                                            let status = format!(
                                                "Web Adapter Config:\n  enabled: {}\n  port: {}\n  token set: {}\n  cors_origins: {}\n  static_dir: {}",
                                                wc.enabled, wc.port, token_set, wc.cors_origins, wc.static_dir
                                            );
                                            app.update(Action::PushAssistantMessage(status));
                                        }
                                        WebCommand::Setup => {
                                            let wc = &config.channels.web;
                                            app.start_web_config_wizard(
                                                wc.enabled,
                                                wc.token.clone(),
                                                wc.port,
                                                &wc.cors_origins,
                                                &wc.static_dir,
                                            );
                                        }
                                        WebCommand::Invalid(msg) => {
                                            app.update(Action::PushError(msg));
                                        }
                                    }
                                    app.update(Action::ScrollToBottom);
                                    continue;
                                }

                                if let Some(wa_cmd) = parse_whatsapp_command(&message) {
                                    match wa_cmd {
                                        WhatsAppCommand::Setup => {
                                            app.update(Action::OpenWhatsAppSetup);
                                        }
                                        WhatsAppCommand::Status => {
                                            let status = format_whatsapp_status(&config);
                                            app.update(Action::PushAssistantMessage(status));
                                        }
                                        WhatsAppCommand::Invalid(msg) => {
                                            app.update(Action::PushError(msg));
                                        }
                                    }
                                    app.update(Action::ScrollToBottom);
                                    continue;
                                }

                                if app.handle_slash_command(&message) {
                                    debug!(command = %message, "Slash command dispatched");
                                    app.update(Action::ScrollToBottom);
                                    continue;
                                }

                                // Regular user message → spawn agent task
                                debug!(message_len = %message.len(), "Agent task spawned");
                                app.update(Action::PushUserMessage(message.clone()));
                                app.update(Action::SetThinking);
                                app.update(Action::ScrollToBottom);

                                let (prog_tx, prog_rx_new) = mpsc::channel::<ProgressEvent>(64);
                                let rt = Arc::clone(&runtime);
                                let sl = Arc::clone(&skill_loader);
                                let ch = channel_id.clone();
                                let sess = app.session_id.clone();

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
                                // Enter in non-idle state or empty input — dispatch via TEA
                                let actions = app.map_key_event(key);
                                for action in actions {
                                    let cmd = app.update(action);
                                    let returned = execute_command(cmd);
                                    if !matches!(returned, Command::None) {
                                        pending_async_cmd = returned;
                                    }
                                }
                            }
                        } else {
                            // ── Non-Enter keys: full TEA dispatch ────────
                            let actions = app.map_key_event(key);
                            for action in actions {
                                let cmd = app.update(action);
                                let returned = execute_command(cmd);
                                if !matches!(returned, Command::None) {
                                    pending_async_cmd = returned;
                                }
                            }
                        }

                        // ── Post-key side effects (model change, refresh, auth) ──
                        if let Some((new_model, provider_name)) = app.take_pending_model_change() {
                            info!(new_model = %new_model, provider = %provider_name, "Model changed");
                            runtime.set_model(new_model.clone());
                            if provider_name != runtime.active_provider_name() {
                                debug!(target_provider = %provider_name, current_provider = %runtime.active_provider_name(), "Switching provider");
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
                                            debug!(provider = %provider_name, "Provider switch completed (on-demand registration)");
                                        } else {
                                            tracing::warn!(provider = %provider_name, "No credential found for provider");
                                        }
                                    } else {
                                        tracing::warn!(provider = %provider_name, "Unknown provider preset");
                                    }
                                }
                            }
                            let state = crate::config::TuiState {
                                last_model: new_model.clone(),
                                last_provider: provider_name.clone(),
                            };
                            let _ = state.save();
                        }


                        if let Some(web_cfg) = app.take_pending_web_config() {
                            config.channels.web = web_cfg;
                            let _ = config.save_web_section();
                        }
                        if app.take_model_refresh_request() {
                            if model_task.is_some() {
                                app.update(Action::PushError(
                                    "Model sync is already in progress. Please wait."
                                        .to_string(),
                                ));
                            } else if let Some(query) = app.model_browser_query() {
                                app.update(Action::MarkModelRefreshing);
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
                                app.update(Action::ScrollToBottom);
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
                                    app.update(Action::SetOAuthCodeDisplayState {
                                        provider: intent.provider.clone(),
                                    });
                                    app.update(Action::PushAssistantMessage(
                                        "Browser opened. Paste the authorization code from your browser.".to_string(),
                                    ));
                                    app.update(Action::ScrollToBottom);
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
                                app.update(Action::SetAuthValidating(
                                    provider_name.clone(),
                                ));
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
                                    let credential = oauth_cred;
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
                                app.update(Action::ScrollToBottom);
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

                        // ── Handle async commands returned by TEA dispatch ──
                        match pending_async_cmd {
                            Command::None => {}
                            Command::SpawnAgentTask(msg) => {
                                if agent_task.is_none() {
                                    debug!(message_len = %msg.len(), "Async command: SpawnAgentTask");
                                    app.update(Action::PushUserMessage(msg.clone()));
                                    app.update(Action::SetThinking);
                                    app.update(Action::ScrollToBottom);
                                    let (prog_tx, prog_rx_new) = mpsc::channel::<ProgressEvent>(64);
                                    let rt = Arc::clone(&runtime);
                                    let sl = Arc::clone(&skill_loader);
                                    let ch = channel_id.clone();
                                    let sess = app.session_id.clone();
                                    let handle = tokio::spawn(async move {
                                        let skills_ctx = sl.load_context().await;
                                        rt.process_with_progress(&ch, &sess, &msg, Some(&skills_ctx), prog_tx).await
                                    });
                                    agent_task = Some(handle);
                                    progress_rx = Some(prog_rx_new);
                                }
                            }
                            Command::RefreshSidebar => {
                                let memory = runtime.memory().clone();
                                if let Ok(sessions) = memory.list_sessions_with_preview().await {
                                    app.update(super::action::Action::RefreshSessionList(
                                        sessions.into_iter().map(|(id, channel_id, updated_at, preview)| {
                                            super::app::SessionEntry { id, channel_id, updated_at, preview }
                                        }).collect(),
                                    ));
                                }
                            }
                            Command::DeleteSession(sid) => {
                                let memory = runtime.memory().clone();
                                let _ = memory.delete_session(&sid).await;
                                app.update(super::action::Action::RemoveSession(sid.clone()));
                                if sid.as_str() == session_id.as_str() {
                                    session_id = SessionId::new();
                                    app.update(super::action::Action::LoadSession {
                                        session_id: session_id.clone(),
                                        messages: Vec::new(),
                                    });
                                }
                                // Refresh sidebar after deletion
                                let memory = runtime.memory().clone();
                                if let Ok(sessions) = memory.list_sessions_with_preview().await {
                                    app.update(super::action::Action::RefreshSessionList(
                                        sessions.into_iter().map(|(id, channel_id, updated_at, preview)| {
                                            super::app::SessionEntry { id, channel_id, updated_at, preview }
                                        }).collect(),
                                    ));
                                }
                            }
                            Command::LoadSessionFromDb(sid) => {
                                let memory = runtime.memory().clone();
                                if let Ok(messages) = memory.load_session(&sid).await {
                                    session_id = sid.clone();
                                    app.update(super::action::Action::LoadSession {
                                        session_id: sid,
                                        messages,
                                    });
                                }
                            }
                            Command::CreateNewSession => {
                                let new_sid = proto::SessionId::new();
                                app.update(super::action::Action::NewSession(new_sid.clone()));
                                session_id = new_sid;
                            }
                            Command::SaveWhatsAppConfig(wa_config) => {
                                config.channels.whatsapp = wa_config;
                                match config.save() {
                                    Ok(()) => {
                                        app.update(Action::PushAssistantMessage(
                                            "WhatsApp configuration saved to config.toml".to_string(),
                                        ));
                                    }
                                    Err(e) => {
                                        app.update(Action::PushError(
                                            format!("Failed to save config: {e}"),
                                        ));
                                    }
                                }
                                app.update(Action::ScrollToBottom);
                            }
                            Command::CheckWhatsAppPrereqs => {
                                // Check Node.js and bridge deps
                                let node_ok = tokio::process::Command::new("node")
                                    .arg("--version")
                                    .stdout(std::process::Stdio::null())
                                    .stderr(std::process::Stdio::null())
                                    .status()
                                    .await
                                    .map(|s| s.success())
                                    .unwrap_or(false);
                                let bridge_installed = std::path::Path::new("whatsapp-bridge/node_modules").exists();
                                let cmd = app.update(Action::WhatsAppPrereqsChecked { node_ok, bridge_installed });
                                // Handle the returned command (may be InstallWhatsAppBridge or SpawnWhatsAppBridge)
                                match cmd {
                                    Command::InstallWhatsAppBridge => {
                                        let handle = tokio::process::Command::new("npm")
                                            .arg("install")
                                            .current_dir("whatsapp-bridge")
                                            .stdout(std::process::Stdio::null())
                                            .stderr(std::process::Stdio::piped())
                                            .status()
                                            .await;
                                        let result = match handle {
                                            Ok(status) if status.success() => Ok(()),
                                            Ok(status) => Err(format!("npm install exited with {status}")),
                                            Err(e) => Err(format!("Failed to run npm install: {e}")),
                                        };
                                        let cmd2 = app.update(Action::WhatsAppBridgeInstalled(result));
                                        if !matches!(cmd2, Command::SpawnWhatsAppBridge) {
                                            continue;
                                        }
                                    }
                                    Command::SpawnWhatsAppBridge => {}
                                    _ => continue,
                                }
                                // Spawn the WhatsApp bridge subprocess
                                let bridge_path = config.channels.whatsapp.bridge_path.clone()
                                    .unwrap_or_else(|| "whatsapp-bridge/index.js".to_string());
                                let session_dir = config.channels.whatsapp.session_dir.clone();
                                match tokio::process::Command::new("node")
                                    .arg(&bridge_path)
                                    .arg(&session_dir)
                                    .stdout(std::process::Stdio::piped())
                                    .stderr(std::process::Stdio::piped())
                                    .stdin(std::process::Stdio::piped())
                                    .kill_on_drop(true)
                                    .spawn()
                                {
                                    Ok(mut child) => {
                                        let stdout = child.stdout.take().expect("bridge stdout");
                                        let (qr_tx, qr_rx) = mpsc::channel::<String>(4);
                                        let (conn_tx, conn_rx) = mpsc::channel::<(String, String)>(1);
                                        whatsapp_bridge_child = Some(child);
                                        whatsapp_qr_rx = Some(qr_rx);
                                        whatsapp_connected_rx = Some(conn_rx);
                                        // Spawn a task to read bridge stdout JSON lines
                                        tokio::spawn(async move {
                                            let reader = tokio::io::BufReader::new(stdout);
                                            let mut lines = reader.lines();
                                            while let Ok(Some(line)) = lines.next_line().await {
                                                if let Ok(event) = serde_json::from_str::<channels::whatsapp::BridgeEvent>(&line) {
                                                    match event {
                                                        channels::whatsapp::BridgeEvent::Qr { data } => {
                                                            let _ = qr_tx.send(data).await;
                                                        }
                                                        channels::whatsapp::BridgeEvent::Connected { phone, name } => {
                                                            let _ = conn_tx.send((phone, name.unwrap_or_default())).await;
                                                            break;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        app.update(Action::PushError(format!("Failed to spawn WhatsApp bridge: {e}")));
                                        app.update(Action::SetIdle);
                                    }
                                }
                            }
                            Command::InstallWhatsAppBridge => {
                                let handle = tokio::process::Command::new("npm")
                                    .arg("install")
                                    .current_dir("whatsapp-bridge")
                                    .stdout(std::process::Stdio::null())
                                    .stderr(std::process::Stdio::piped())
                                    .status()
                                    .await;
                                let result = match handle {
                                    Ok(status) if status.success() => Ok(()),
                                    Ok(status) => Err(format!("npm install exited with {status}")),
                                    Err(e) => Err(format!("Failed to run npm install: {e}")),
                                };
                                app.update(Action::WhatsAppBridgeInstalled(result));
                            }
                            Command::SpawnWhatsAppBridge => {
                                let bridge_path = config.channels.whatsapp.bridge_path.clone()
                                    .unwrap_or_else(|| "whatsapp-bridge/index.js".to_string());
                                let session_dir = config.channels.whatsapp.session_dir.clone();
                                match tokio::process::Command::new("node")
                                    .arg(&bridge_path)
                                    .arg(&session_dir)
                                    .stdout(std::process::Stdio::piped())
                                    .stderr(std::process::Stdio::piped())
                                    .stdin(std::process::Stdio::piped())
                                    .kill_on_drop(true)
                                    .spawn()
                                {
                                    Ok(mut child) => {
                                        let stdout = child.stdout.take().expect("bridge stdout");
                                        let (qr_tx, qr_rx) = mpsc::channel::<String>(4);
                                        let (conn_tx, conn_rx) = mpsc::channel::<(String, String)>(1);
                                        whatsapp_bridge_child = Some(child);
                                        whatsapp_qr_rx = Some(qr_rx);
                                        whatsapp_connected_rx = Some(conn_rx);
                                        tokio::spawn(async move {
                                            let reader = tokio::io::BufReader::new(stdout);
                                            let mut lines = reader.lines();
                                            while let Ok(Some(line)) = lines.next_line().await {
                                                if let Ok(event) = serde_json::from_str::<channels::whatsapp::BridgeEvent>(&line) {
                                                    match event {
                                                        channels::whatsapp::BridgeEvent::Qr { data } => {
                                                            let _ = qr_tx.send(data).await;
                                                        }
                                                        channels::whatsapp::BridgeEvent::Connected { phone, name } => {
                                                            let _ = conn_tx.send((phone, name.unwrap_or_default())).await;
                                                            break;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        app.update(Action::PushError(format!("Failed to spawn WhatsApp bridge: {e}")));
                                        app.update(Action::SetIdle);
                                    }
                                }
                            }
                            _ => {
                                // Other commands (CopyToClipboard, Batch) already handled
                                // by execute_command; StartAuthFlow, LoadModelCatalog
                                // handled via post-key side effects.
                            }
                        }
                    }
                    Some(Ok(Event::Mouse(mouse))) => {
                        // ── Mouse events: full TEA dispatch ──────────────
                        let frame_area: ratatui::layout::Rect = terminal.size().unwrap_or_default().into();
                        let actions = app.map_mouse_event(mouse, frame_area);
                        for action in actions {
                            let cmd = app.update(action);
                            let _ = execute_command(cmd);
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => {
                        app.update(Action::Resize);
                    }
                    Some(Err(_)) | None => {
                        break; // stream ended or error
                    }
                    _ => {}
                }
            }

            // ── Branch 2: progress events from agent task ────────
            Some(evt) = async {
                match progress_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                app.update(super::action::Action::ApplyProgress(evt));
            }

            // ── Branch 3: agent task completed ───────────────────
            result = async {
                match agent_task.as_mut() {
                    Some(handle) => handle.await,
                    None => std::future::pending().await,
                }
            } => {
                let completion_result = match result {
                    Ok(inner) => {
                        debug!(success = %inner.is_ok(), "Agent task completed");
                        inner.map_err(|e| e.to_string())
                    }
                    Err(join_err) => Err(format!("Task panicked: {join_err}")),
                };
                app.update(super::action::Action::ApplyCompletion(completion_result));
                agent_task = None;
                progress_rx = None;
                // Refresh sidebar after agent completion
                let memory = runtime.memory().clone();
                if let Ok(sessions) = memory.list_sessions_with_preview().await {
                    app.update(super::action::Action::RefreshSessionList(
                        sessions
                            .into_iter()
                            .map(|(id, channel_id, updated_at, preview)| {
                                super::app::SessionEntry {
                                    id,
                                    channel_id,
                                    updated_at,
                                    preview,
                                }
                            })
                            .collect(),
                    ));
                }
            }

            // ── Branch 4: auth task completed ────────────────────
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
                        // Persist last used model after auth
                        if let Some(ref provider_str) = auth_provider_name {
                            let model = provider_str.parse::<ProviderPreset>()
                                .map(|p| p.default_model().to_string())
                                .unwrap_or_default();
                            let state = crate::config::TuiState {
                                last_model: model,
                                last_provider: provider_str.clone(),
                            };
                            let _ = state.save();
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
                        app.update(super::action::Action::PushAssistantMessage(message));
                    }
                    Ok(Err(err)) => {
                        debug!(provider = ?auth_provider_name, error = %err, "Auth task failed");
                        if let Some((old_preset, old_name)) = prev_provider.take() {
                            config.agent.provider = old_preset;
                            app.provider_name = old_name;
                        }
                        auth_provider_name = None;
                        app.update(super::action::Action::PushError(format!("Authentication failed: {err}")));
                    }
                    Err(join_err) => {
                        debug!(provider = ?auth_provider_name, error = %join_err, "Auth task panicked");
                        if let Some((old_preset, old_name)) = prev_provider.take() {
                            config.agent.provider = old_preset;
                            app.provider_name = old_name;
                        }
                        auth_provider_name = None;
                        app.update(super::action::Action::PushError(format!("Auth task failed: {join_err}")));
                    }
                }
                app.update(Action::SetIdle);
                app.update(Action::ScrollToBottom);
                auth_task = None;
            }

            // ── Branch 5: model task completed ────────────────────────
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
                                app.update(super::action::Action::OpenModelBrowser {
                                    provider: provider_label,
                                    entries: catalog.entries,
                                    query,
                                    sync_status: sync_status_combined,
                                });
                            }
                            Some(ModelTaskMode::List) => {
                                let text = format_model_list(&catalog.entries, &catalog.sync_statuses);
                                app.update(super::action::Action::PushAssistantMessage(text));
                            }
                            None => {
                                // background pre-cache only — no browser opened
                            }
                        }
                    }
                    Err(join_err) => {
                        debug!(error = %join_err, "Model task failed");
                        app.update(super::action::Action::PushError(format!("Model task failed: {join_err}")));
                    }
                }
                model_task = None;
                app.update(super::action::Action::ScrollToBottom);
            }

            // ── Branch: tool approval request ─────────────────────────
            Some(pending) = approval_rx.recv() => {
                app.pending_approval = Some(pending);
            }

            // ── Branch 6: spinner tick ─────────────────────────────────
            _ = spinner_interval.tick(), if app.state != AppState::Idle => {
                app.update(super::action::Action::Tick);
            }

            // ── Branch 7: WhatsApp QR code received ──────────────
            Some(qr_data) = async {
                match whatsapp_qr_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                app.update(super::action::Action::WhatsAppQrReceived(qr_data));
            }

            // ── Branch 8: WhatsApp connected ─────────────────────
            Some((phone, name)) = async {
                match whatsapp_connected_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let cmd = app.update(super::action::Action::WhatsAppConnected { phone, name });
                // Handle SaveWhatsAppConfig command
                if let super::action::Command::SaveWhatsAppConfig(wa_config) = cmd {
                    config.channels.whatsapp = wa_config;
                    match config.save() {
                        Ok(()) => {
                            app.update(super::action::Action::PushAssistantMessage(
                                "WhatsApp configuration saved to config.toml".to_string(),
                            ));
                        }
                        Err(e) => {
                            app.update(super::action::Action::PushError(
                                format!("Failed to save config: {e}"),
                            ));
                        }
                    }
                    app.update(super::action::Action::ScrollToBottom);
                }
                // Clean up bridge receivers (keep child alive for daemon use)
                whatsapp_qr_rx = None;
                whatsapp_connected_rx = None;
            }
        }

        // ── Post-select: sidebar session selection ─────────────────
        if let Some(new_session_id) = app.take_pending_sidebar_selection() {
            let memory = runtime.memory().clone();
            if let Ok(messages) = memory.load_session(&new_session_id).await {
                session_id = new_session_id.clone();
                app.update(super::action::Action::LoadSession {
                    session_id: new_session_id,
                    messages,
                });
            }
        }

        // ── Post-select: session browser new request ──────────────
        if app.session_browser_new_requested {
            app.session_browser_new_requested = false;
            let new_sid = proto::SessionId::new();
            app.update(super::action::Action::NewSession(new_sid.clone()));
            session_id = new_sid;
        }

        // ── Post-select: confirmed session deletion ───────────────
        if let Some(del_id) = app.take_confirmed_delete() {
            let memory = runtime.memory().clone();
            let _ = memory.delete_session(&del_id).await;
            app.update(super::action::Action::RemoveSession(del_id.clone()));
            // If we deleted the active session, create a new one
            if del_id.as_str() == session_id.as_str() {
                session_id = SessionId::new();
                app.update(super::action::Action::LoadSession {
                    session_id: session_id.clone(),
                    messages: Vec::new(),
                });
            }
            // Refresh sidebar
            if let Ok(sessions) = memory.list_sessions_with_preview().await {
                app.update(super::action::Action::RefreshSessionList(
                    sessions
                        .into_iter()
                        .map(
                            |(id, channel_id, updated_at, preview)| super::app::SessionEntry {
                                id,
                                channel_id,
                                updated_at,
                                preview,
                            },
                        )
                        .collect(),
                ));
            }
        }

        // ── Post-select: WhatsApp bridge cleanup on cancel ────
        if matches!(app.state, AppState::Idle) && whatsapp_bridge_child.is_some() {
            // If state went back to Idle while bridge was running (user cancelled),
            // kill the bridge
            if let Some(mut child) = whatsapp_bridge_child.take() {
                let _ = child.kill().await;
            }
            whatsapp_qr_rx = None;
            whatsapp_connected_rx = None;
        }

        if app.should_quit {
            // Kill WhatsApp bridge on exit
            if let Some(mut child) = whatsapp_bridge_child.take() {
                let _ = child.kill().await;
            }
            break;
        }
    }

    // TerminalGuard::drop handles cleanup
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restore_home_env(original_home: Option<String>) {
        match original_home {
            Some(home) => crate::test_support::set_env_var("HOME", &home),
            None => crate::test_support::remove_env_var("HOME"),
        }
    }

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
    fn parse_session_command_supports_all_variants() {
        // bare /session => List
        assert_eq!(
            parse_session_command("/session"),
            Some(SessionCommand::List)
        );
        assert_eq!(
            parse_session_command("/session list"),
            Some(SessionCommand::List)
        );

        // new
        assert_eq!(
            parse_session_command("/session new"),
            Some(SessionCommand::New)
        );

        // load requires id
        assert_eq!(
            parse_session_command("/session load"),
            Some(SessionCommand::Invalid(
                "Usage: /session load <id>".to_string()
            ))
        );
        assert_eq!(
            parse_session_command("/session load abc123"),
            Some(SessionCommand::Load("abc123".to_string()))
        );

        // delete / del
        assert_eq!(
            parse_session_command("/session delete"),
            Some(SessionCommand::Invalid(
                "Usage: /session delete <id>".to_string()
            ))
        );
        assert_eq!(
            parse_session_command("/session delete xyz"),
            Some(SessionCommand::Delete("xyz".to_string()))
        );
        assert_eq!(
            parse_session_command("/session del xyz"),
            Some(SessionCommand::Delete("xyz".to_string()))
        );

        // unknown subcommand
        assert!(matches!(
            parse_session_command("/session foobar"),
            Some(SessionCommand::Invalid(_))
        ));

        // non-session command returns None
        assert_eq!(parse_session_command("/model"), None);
        assert_eq!(parse_session_command("/help"), None);
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

    #[test]
    fn format_model_list_recommended_and_other() {
        use model_catalog::{ModelCatalogEntry, ModelSource, ModelStatus};
        let entries = vec![
            ModelCatalogEntry {
                id: "gpt-4o".into(),
                provider: "openai".into(),
                recommended_for_coding: true,
                status: ModelStatus::Stable,
                source: ModelSource::Docs,
                available: true,
            },
            ModelCatalogEntry {
                id: "llama3".into(),
                provider: "ollama".into(),
                recommended_for_coding: false,
                status: ModelStatus::Stable,
                source: ModelSource::Api,
                available: true,
            },
            ModelCatalogEntry {
                id: "old-model".into(),
                provider: "x".into(),
                recommended_for_coding: true,
                status: ModelStatus::Unknown,
                source: ModelSource::Docs,
                available: false,
            },
        ];
        let out = format_model_list(&entries, &["openai: ok".into()]);
        assert!(out.contains("Models \u{2014} 3 total"));
        assert!(out.contains("Recommended:"));
        assert!(out.contains("\u{2605}  gpt-4o [openai]"));
        assert!(out.contains("Other:"));
        assert!(out.contains("llama3 [ollama] (api)"));
        assert!(out.contains("Sync: openai: ok"));
        assert!(!out.contains("old-model"));
    }

    #[test]
    fn format_model_list_empty() {
        let out = format_model_list(&[], &[]);
        assert!(out.contains("Models \u{2014} 0 total"));
        assert!(!out.contains("Recommended"));
        assert!(!out.contains("Other"));
        assert!(!out.contains("Sync"));
    }

    #[test]
    fn format_model_list_only_recommended() {
        use model_catalog::{ModelCatalogEntry, ModelSource, ModelStatus};
        let entries = vec![ModelCatalogEntry {
            id: "claude".into(),
            provider: "anthropic".into(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Api,
            available: true,
        }];
        let out = format_model_list(&entries, &[]);
        assert!(out.contains("Recommended:"));
        assert!(out.contains("\u{2605}  claude [anthropic] (api)"));
        assert!(!out.contains("Other:"));
    }

    #[test]
    fn format_model_list_multiple_sync_statuses() {
        let out = format_model_list(&[], &["a: ok".into(), "b: fail".into()]);
        assert!(out.contains("Sync: a: ok; b: fail"));
    }

    #[test]
    fn collect_authenticated_providers_default_config() {
        let config = Config::default();
        let providers = collect_authenticated_providers(&config);
        assert!(providers.is_empty() || providers.iter().all(|(_, _, k)| !k.is_empty()));
    }

    #[test]
    fn model_sync_in_progress_error_message_is_stable() {
        assert_eq!(
            model_sync_in_progress_error(),
            "Model sync is already in progress. Please wait."
        );
    }

    #[test]
    fn collect_authenticated_providers_does_not_duplicate_active_provider() {
        crate::test_support::with_locked_env(|| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let original_home = std::env::var("HOME").ok();
            crate::test_support::set_env_var("HOME", tmp.path().to_str().expect("utf8 path"));

            let mut creds = crate::auth::Credentials::default();
            creds.set(
                "openai".to_string(),
                crate::auth::ProviderCredential {
                    access_token: "tok-openai".to_string(),
                    endpoint: None,
                    refresh_token: None,
                    expires_at: None,
                    id_token: None,
                },
            );
            let creds_path = crate::auth::Credentials::path();
            creds.save_to(&creds_path).expect("save creds");

            let config = Config::default();
            let providers = collect_authenticated_providers(&config);
            let active = config.agent.provider.name().to_string();
            let active_count = providers
                .iter()
                .filter(|(name, _, _)| name == &active)
                .count();
            assert_eq!(active_count, 1);

            restore_home_env(original_home);
        });
    }

    #[test]
    fn restore_home_env_clears_home_when_original_missing() {
        crate::test_support::with_locked_env(|| {
            crate::test_support::set_env_var("HOME", "/tmp/openpista-event-home");
            restore_home_env(None);
            assert!(std::env::var("HOME").is_err());
        });
    }

    #[test]
    fn restore_home_env_sets_home_when_original_present() {
        crate::test_support::with_locked_env(|| {
            restore_home_env(Some("/tmp/openpista-event-home-restored".to_string()));
            assert_eq!(
                std::env::var("HOME").as_deref(),
                Ok("/tmp/openpista-event-home-restored")
            );
        });
    }

    #[tokio::test]
    async fn maybe_exchange_copilot_token_passthrough_for_copilot() {
        let cred = crate::auth::ProviderCredential {
            access_token: "gh_token_123".into(),
            refresh_token: None,
            expires_at: None,
            endpoint: None,
            id_token: None,
        };
        let result = maybe_exchange_copilot_token("github-copilot", cred).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().access_token, "gh_token_123");
    }

    #[tokio::test]
    async fn maybe_exchange_copilot_token_passthrough_for_other_provider() {
        let cred = crate::auth::ProviderCredential {
            access_token: "some_token".into(),
            refresh_token: None,
            expires_at: None,
            endpoint: None,
            id_token: None,
        };
        let result = maybe_exchange_copilot_token("openai", cred).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().access_token, "some_token");
    }

    #[test]
    fn parse_whatsapp_command_supports_all_variants() {
        assert_eq!(
            parse_whatsapp_command("/whatsapp"),
            Some(WhatsAppCommand::Setup)
        );
        assert_eq!(
            parse_whatsapp_command("/whatsapp setup"),
            Some(WhatsAppCommand::Setup)
        );
        assert_eq!(
            parse_whatsapp_command("/whatsapp status"),
            Some(WhatsAppCommand::Status)
        );
        assert!(matches!(
            parse_whatsapp_command("/whatsapp foo"),
            Some(WhatsAppCommand::Invalid(_))
        ));
        assert_eq!(parse_whatsapp_command("/model"), None);
        assert_eq!(parse_whatsapp_command("/help"), None);
    }

    #[test]
    fn format_whatsapp_status_shows_defaults() {
        let config = Config::default();
        let status = format_whatsapp_status(&config);
        assert!(status.contains("WhatsApp Configuration Status"));
        assert!(status.contains("No"));
        assert!(status.contains("(bundled default)"));
        assert!(status.contains("Ready"));
    }

    #[test]
    fn format_whatsapp_status_configured() {
        let mut config = Config::default();
        config.channels.whatsapp.enabled = true;
        config.channels.whatsapp.session_dir = "/tmp/wa-session".to_string();
        config.channels.whatsapp.bridge_path = Some("/opt/bridge/index.js".to_string());
        let status = format_whatsapp_status(&config);
        assert!(status.contains("Yes"));
        assert!(status.contains("/tmp/wa-session"));
        assert!(status.contains("/opt/bridge/index.js"));
        assert!(status.contains("Ready"));
    }

    #[test]
    fn render_qr_text_produces_valid_output() {
        let qr = render_qr_text("https://wa.me/123456789");
        assert!(qr.is_some());
        let text = qr.unwrap();
        assert!(!text.is_empty());
        // Should contain block characters used for QR rendering
        assert!(
            text.contains('\u{2588}') || text.contains('\u{2580}') || text.contains('\u{2584}')
        );
        // Should have multiple lines
        assert!(text.lines().count() > 5);
    }

    #[test]
    fn render_qr_text_empty_url_still_works() {
        // Even an empty string should produce a valid QR code
        let qr = render_qr_text("");
        assert!(qr.is_some());
    }

    #[test]
    fn detect_local_ip_returns_non_empty_string() {
        let ip = detect_local_ip();
        assert!(!ip.is_empty());
        // Must be either a valid IP or the fallback
        assert!(ip == "localhost" || ip.contains('.') || ip.contains(':'));
    }

    #[test]
    fn format_model_list_other_with_docs_source_no_api_tag() {
        use model_catalog::{ModelCatalogEntry, ModelSource, ModelStatus};
        let entries = vec![ModelCatalogEntry {
            id: "old-model".into(),
            provider: "together".into(),
            recommended_for_coding: false,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: true,
        }];
        let out = format_model_list(&entries, &[]);
        assert!(out.contains("Other:"));
        assert!(out.contains("old-model [together]"));
        // Docs-source entries in Other should NOT have the "(api)" tag
        assert!(!out.contains("(api)"));
    }

    #[test]
    fn collect_authenticated_providers_includes_active_when_api_key_set() {
        let mut config = Config::default();
        config.agent.api_key = "test-key-abc".to_string();
        let providers = collect_authenticated_providers(&config);
        // Active provider with a configured key should be included
        let active = config.agent.provider.name().to_string();
        let found = providers
            .iter()
            .any(|(n, _, k)| n == &active && !k.is_empty());
        assert!(
            found,
            "active provider should appear when api_key is configured"
        );
    }

    // ── build_and_store_credential_with_path error paths ─────────────

    #[tokio::test]
    async fn persist_auth_unknown_provider_returns_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "nonexistent-provider-xyz".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: Some("key".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Unknown provider"), "got: {err}");
    }

    #[tokio::test]
    async fn persist_auth_api_key_empty_returns_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "together".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: Some("   ".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("empty"), "got: {err}");
    }

    #[tokio::test]
    async fn persist_auth_api_key_with_whitespace_returns_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "together".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: Some("key with spaces".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("whitespace"), "got: {err}");
    }

    #[tokio::test]
    async fn persist_auth_api_key_missing_returns_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "together".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: None,
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("API key input is required"), "got: {err}");
    }

    #[tokio::test]
    async fn persist_auth_endpoint_and_key_missing_endpoint_returns_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "custom".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: Some("tok-custom".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Endpoint is required"), "got: {err}");
    }

    #[tokio::test]
    async fn persist_auth_endpoint_and_key_empty_endpoint_returns_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "custom".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: Some("   ".to_string()),
                api_key: Some("tok-custom".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Endpoint is required"), "got: {err}");
    }

    #[tokio::test]
    async fn persist_auth_ollama_no_auth_returns_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "ollama".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: None,
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path,
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("does not require authentication"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn persist_auth_oauth_api_key_fallback_saves_credential() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "openai".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: Some("sk-openai-test".to_string()),
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path.clone(),
        )
        .await;
        assert!(result.is_ok(), "got: {:?}", result);
        let msg = result.unwrap();
        assert!(msg.contains("Saved API key"), "got: {msg}");
        let content = std::fs::read_to_string(&cred_path).expect("read");
        let creds: crate::auth::Credentials = toml::from_str(&content).expect("parse");
        let saved = creds.get("openai").expect("credential saved");
        assert_eq!(saved.access_token, "sk-openai-test");
    }

    #[tokio::test]
    async fn persist_auth_openai_oauth_fails_in_test_mode() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        let result = persist_auth_with_path(
            Config::default(),
            AuthLoginIntent {
                provider: "openai".to_string(),
                auth_method: AuthMethodChoice::OAuth,
                endpoint: None,
                api_key: None,
            },
            OAUTH_CALLBACK_PORT,
            OAUTH_TIMEOUT_SECS,
            cred_path,
        )
        .await;
        // OAuth is stubbed in test mode, should fail
        assert!(result.is_err());
    }

    #[test]
    fn validate_api_key_trims_whitespace() {
        assert_eq!(
            validate_api_key("  sk-test  ".to_string()).unwrap(),
            "sk-test"
        );
    }

    #[test]
    fn load_credentials_returns_default_for_missing_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nonexistent.toml");
        let creds = load_credentials(&path);
        assert!(creds.get("openai").is_none());
    }

    #[test]
    fn load_credentials_returns_default_for_invalid_toml() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("bad.toml");
        std::fs::write(&path, "not valid toml {{{").unwrap();
        let creds = load_credentials(&path);
        assert!(creds.get("openai").is_none());
    }

    #[test]
    fn persist_credential_creates_file_and_saves() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("creds.toml");
        let cred = crate::auth::ProviderCredential {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: None,
            endpoint: None,
            id_token: None,
        };
        persist_credential("test-provider".to_string(), cred, path.clone()).unwrap();
        let loaded = load_credentials(&path);
        let saved = loaded.get("test-provider").expect("should exist");
        assert_eq!(saved.access_token, "tok");
    }

    #[test]
    fn format_whatsapp_status_unconfigured_session_dir() {
        let mut config = Config::default();
        config.channels.whatsapp.session_dir = String::new();
        let status = format_whatsapp_status(&config);
        assert!(status.contains("Incomplete"));
    }

    #[test]
    fn render_qr_text_long_url_produces_larger_qr() {
        let short = render_qr_text("hi").unwrap();
        let long =
            render_qr_text("https://example.com/very/long/path/to/resource?with=params&and=more")
                .unwrap();
        assert!(long.lines().count() >= short.lines().count());
    }

    #[test]
    fn format_model_list_only_unavailable_models_shows_nothing() {
        use model_catalog::{ModelCatalogEntry, ModelSource, ModelStatus};
        let entries = vec![ModelCatalogEntry {
            id: "unavailable-model".into(),
            provider: "test".into(),
            recommended_for_coding: true,
            status: ModelStatus::Stable,
            source: ModelSource::Docs,
            available: false,
        }];
        let out = format_model_list(&entries, &[]);
        assert!(out.contains("1 total"));
        // Unavailable models are not shown in either section
        assert!(!out.contains("Recommended:"));
        assert!(!out.contains("Other:"));
    }

    #[test]
    fn collect_authenticated_providers_returns_vec() {
        let mut config = Config::default();
        config.agent.api_key = "test-key-for-loop".to_string();
        let providers = collect_authenticated_providers(&config);
        // Result is always a Vec; each entry has (name, optional url, key)
        for (name, _url, _key) in &providers {
            assert!(!name.is_empty(), "provider name must not be empty");
        }
    }
}
