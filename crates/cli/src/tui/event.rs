//! Async event loop for the TUI — interleaves crossterm, agent progress, and timer events.
#![allow(dead_code, unused_imports)]

use std::str::FromStr;
use std::sync::Arc;

use crossterm::{
    event::{Event, EventStream, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use proto::{ChannelId, ProgressEvent, SessionId};
use ratatui::{Terminal, backend::CrosstermBackend};
use skills::SkillLoader;
use tokio::sync::mpsc;

use super::app::{AppState, TuiApp};
use crate::auth_picker::{AuthLoginIntent, AuthMethodChoice};
use crate::config::{LoginAuthMode, OAuthEndpoints, ProviderPreset, provider_registry_entry};
use crate::model_catalog;

const OAUTH_CALLBACK_PORT: u16 = 9009;
const OAUTH_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelsCommand {
    Browse,
    Invalid(String),
}

fn parse_models_command(raw: &str) -> Option<ModelsCommand> {
    let mut parts = raw.split_whitespace();
    if parts.next()? != "/models" {
        return None;
    }

    if parts.next().is_none() {
        Some(ModelsCommand::Browse)
    } else {
        Some(ModelsCommand::Invalid(
            "Use /models, then press s to search and r to refresh.".to_string(),
        ))
    }
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
) -> Result<(), String> {
    let mut creds = crate::auth::Credentials::load();
    creds.set(provider, credential);
    creds
        .save_to(&credentials_path())
        .map_err(|e| e.to_string())
}

fn credentials_path() -> std::path::PathBuf {
    std::env::var("OPENPISTACRAB_CREDENTIALS_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| crate::auth::Credentials::path())
}

#[cfg(not(test))]
/// Runs browser OAuth login flow for one provider.
async fn run_oauth_login(
    provider: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
) -> anyhow::Result<crate::auth::ProviderCredential> {
    crate::auth::login(
        provider,
        endpoints,
        client_id,
        OAUTH_CALLBACK_PORT,
        OAUTH_TIMEOUT_SECS,
    )
    .await
}

#[cfg(test)]
/// Test stub for OAuth flow; keeps tests deterministic.
async fn run_oauth_login(
    _provider: &str,
    endpoints: &OAuthEndpoints,
    client_id: &str,
) -> anyhow::Result<crate::auth::ProviderCredential> {
    let _ = (
        endpoints.auth_url,
        endpoints.token_url,
        endpoints.scope,
        client_id,
    );
    anyhow::bail!("OAuth login is not available in tests")
}

/// Persists authentication data for OAuth/API-key login paths.
async fn persist_auth(intent: AuthLoginIntent, oauth_client_id: String) -> Result<String, String> {
    let provider = intent.provider.to_ascii_lowercase();
    let entry = provider_registry_entry(&provider)
        .ok_or_else(|| format!("Unknown provider '{provider}'"))?;
    let provider_name = entry.name.to_string();
    let resolved_method = if entry.name == "openai" {
        intent.auth_method
    } else {
        AuthMethodChoice::ApiKey
    };

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
                let preset = ProviderPreset::from_str(entry.name).map_err(|_| {
                    format!(
                        "Provider '{provider_name}' is an extension slot and does not yet support runtime OAuth"
                    )
                })?;

                let endpoints = preset.oauth_endpoints().ok_or_else(|| {
                    format!("Provider '{provider_name}' does not support OAuth login")
                })?;

                let client_id = oauth_client_id.trim().to_string();
                if client_id.is_empty() {
                    return Err(
                        "No OAuth client ID configured. Set OPENPISTACRAB_OAUTH_CLIENT_ID or agent.oauth_client_id."
                            .to_string(),
                    );
                }

                let credential = run_oauth_login(&provider_name, &endpoints, &client_id)
                    .await
                    .map_err(|e| e.to_string())?;
                (
                    credential,
                    format!(
                        "Authenticated as '{provider_name}'. Token stored in {}",
                        credentials_path().display()
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

    tokio::task::spawn_blocking(move || persist_credential(provider_name, credential))
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

/// RAII guard that restores the terminal on drop (even on panic).
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    }
}

/// Run the full-screen TUI until the user quits.
pub async fn run_tui(
    runtime: Arc<agent::AgentRuntime>,
    skill_loader: Arc<SkillLoader>,
    channel_id: ChannelId,
    session_id: SessionId,
    model_name: String,
    oauth_client_id: String,
) -> anyhow::Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _guard = TerminalGuard; // Drop restores terminal

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // App state
    let mut app = TuiApp::new(&model_name, session_id.clone(), channel_id.clone());

    // Crossterm event stream (async)
    let mut crossterm_stream = EventStream::new();

    // Agent task state
    let mut agent_task: Option<tokio::task::JoinHandle<Result<String, proto::Error>>> = None;
    let mut progress_rx: Option<mpsc::Receiver<ProgressEvent>> = None;
    let mut auth_task: Option<tokio::task::JoinHandle<Result<String, String>>> = None;
    let mut model_task: Option<tokio::task::JoinHandle<model_catalog::CatalogLoadResult>> = None;
    let mut model_task_opts: Option<String> = None;

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
                                            app.open_model_browser(
                                                model_catalog::OPENCODE_PROVIDER.to_string(),
                                                Vec::new(),
                                                String::new(),
                                                "Loading models...".to_string(),
                                            );
                                            model_task_opts = Some(String::new());
                                            model_task = Some(tokio::spawn(async move {
                                                model_catalog::load_opencode_catalog(false).await
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
                                    app.scroll_to_bottom();
                                    continue;
                                }
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

                        if app.take_model_refresh_request() {
                            if model_task.is_some() {
                                app.push_error(
                                    "Model sync is already in progress. Please wait."
                                        .to_string(),
                                );
                            } else if let Some(query) = app.model_browser_query() {
                                app.mark_model_refreshing();
                                model_task_opts = Some(query);
                                model_task = Some(tokio::spawn(async move {
                                    model_catalog::load_opencode_catalog(true).await
                                }));
                            }
                        }

                        if auth_task.is_none()
                            && let Some(intent) = app.take_pending_auth_intent()
                        {
                            if intent.provider == "openai"
                                && intent.auth_method == AuthMethodChoice::OAuth
                                && oauth_client_id.trim().is_empty()
                            {
                                app.reopen_openai_method_with_error(
                                    "No OAuth client ID configured. Choose API key mode or set OPENPISTACRAB_OAUTH_CLIENT_ID.".to_string(),
                                );
                                app.scroll_to_bottom();
                                continue;
                            }
                            let oauth_client_id_for_task = oauth_client_id.clone();
                            auth_task = Some(tokio::spawn(async move {
                                persist_auth(intent, oauth_client_id_for_task).await
                            }));
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
                    Ok(inner) => app.apply_completion(inner),
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
                        app.push_assistant(message);
                    }
                    Ok(Err(err)) => {
                        app.push_error(format!("Authentication failed: {err}"));
                    }
                    Err(join_err) => app.push_error(format!("Auth task failed: {join_err}")),
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
                        let query = model_task_opts.take().unwrap_or_default();
                        app.open_model_browser(
                            catalog.provider,
                            catalog.entries,
                            query,
                            catalog.sync_status,
                        );
                    }
                    Err(join_err) => {
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
    use std::sync::OnceLock;
    use tokio::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn set_env_var(key: &str, value: &str) {
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_env_var(key: &str) {
        unsafe {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn terminal_guard_drop_path_is_safe() {
        let guard = TerminalGuard;
        drop(guard);
    }

    #[test]
    fn parse_models_command_supports_browse_variants() {
        assert_eq!(parse_models_command("/models"), Some(ModelsCommand::Browse));
        assert_eq!(
            parse_models_command("/models all"),
            Some(ModelsCommand::Invalid(
                "Use /models, then press s to search and r to refresh.".to_string()
            ))
        );
        assert_eq!(
            parse_models_command("/models refresh"),
            Some(ModelsCommand::Invalid(
                "Use /models, then press s to search and r to refresh.".to_string()
            ))
        );
        assert_eq!(
            parse_models_command("/models search codex"),
            Some(ModelsCommand::Invalid(
                "Use /models, then press s to search and r to refresh.".to_string()
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
        let _guard = env_lock().lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        set_env_var(
            "OPENPISTACRAB_CREDENTIALS_PATH",
            cred_path.to_string_lossy().as_ref(),
        );

        let result = persist_auth(
            AuthLoginIntent {
                provider: "together".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: None,
                api_key: Some("tok-together".to_string()),
            },
            String::new(),
        )
        .await;

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&cred_path).expect("read credentials");
        let creds: crate::auth::Credentials = toml::from_str(&content).expect("parse credentials");
        let saved = creds.get("together").expect("credential saved");
        assert_eq!(saved.access_token, "tok-together");
        assert_eq!(saved.endpoint, None);

        remove_env_var("OPENPISTACRAB_CREDENTIALS_PATH");
    }

    #[tokio::test]
    async fn persist_auth_endpoint_and_key_saves_endpoint() {
        let _guard = env_lock().lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let cred_path = tmp.path().join("credentials.toml");
        set_env_var(
            "OPENPISTACRAB_CREDENTIALS_PATH",
            cred_path.to_string_lossy().as_ref(),
        );

        let result = persist_auth(
            AuthLoginIntent {
                provider: "azure-openai".to_string(),
                auth_method: AuthMethodChoice::ApiKey,
                endpoint: Some("https://example.azure.com".to_string()),
                api_key: Some("tok-azure".to_string()),
            },
            String::new(),
        )
        .await;

        assert!(result.is_ok());
        let content = std::fs::read_to_string(&cred_path).expect("read credentials");
        let creds: crate::auth::Credentials = toml::from_str(&content).expect("parse credentials");
        let saved = creds.get("azure-openai").expect("credential saved");
        assert_eq!(saved.access_token, "tok-azure");
        assert_eq!(saved.endpoint.as_deref(), Some("https://example.azure.com"));

        remove_env_var("OPENPISTACRAB_CREDENTIALS_PATH");
    }

    #[tokio::test]
    async fn persist_auth_oauth_requires_client_id() {
        let err = persist_auth(
            AuthLoginIntent {
                provider: "openai".to_string(),
                auth_method: AuthMethodChoice::OAuth,
                endpoint: None,
                api_key: None,
            },
            String::new(),
        )
        .await
        .expect_err("missing oauth client id should fail");
        assert!(err.contains("No OAuth client ID configured"));
    }
}
