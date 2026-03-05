//! Web adapter subcommand handlers and helpers.

#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
#[cfg(not(test))]
use rand::{RngCore, rngs::OsRng};
#[cfg(not(test))]
use std::io::{IsTerminal, Write};

#[cfg(not(test))]
use agent::{AgentRuntime, AutoApproveHandler, Memory};
#[cfg(not(test))]
use channels::{ChannelAdapter, SessionLoader, WebAdapter, WebModelEntry, WebSessionEntry};
#[cfg(not(test))]
use proto::{AgentResponse, ChannelEvent};
#[cfg(not(test))]
use skills::SkillLoader;
#[cfg(not(test))]
use tracing::{debug, error, info, warn};

#[cfg(not(test))]
use crate::config::Config;
#[cfg(not(test))]
use crate::startup::{build_provider, build_runtime};

#[cfg(not(test))]
pub(crate) struct WebSetupOptions {
    pub token: Option<String>,
    pub regenerate_token: bool,
    pub yes: bool,
    pub port: Option<u16>,
    pub cors_origins: Option<String>,
    pub static_dir: Option<String>,
    pub shared_session_id: Option<String>,
    pub enable: bool,
    pub disable: bool,
}

/// Bridges `Memory` to the `SessionLoader` trait expected by `WebAdapter`.
#[cfg(not(test))]
struct MemorySessionLoader(Arc<dyn Memory>);

#[cfg(not(test))]
#[async_trait::async_trait]
impl SessionLoader for MemorySessionLoader {
    async fn load_session_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<proto::AgentMessage>, String> {
        self.0
            .load_session(&proto::SessionId::from(session_id))
            .await
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(test))]
pub(crate) fn build_web_model_list() -> Vec<WebModelEntry> {
    use crate::model_catalog::seed_models_for_provider;

    let mut web_models = Vec::new();
    for provider_name in &["openai", "anthropic", "together", "ollama", "openrouter"] {
        for entry in seed_models_for_provider(provider_name) {
            web_models.push(WebModelEntry {
                provider: provider_name.to_string(),
                model: entry.id.clone(),
                recommended: entry.recommended_for_coding,
            });
        }
    }
    web_models
}

/// Resolves OAuth endpoints for a provider — checks runtime presets first, then extensions.
#[cfg(not(test))]
pub(crate) fn resolve_oauth_endpoints(
    provider_name: &str,
) -> Option<crate::config::OAuthEndpoints> {
    use crate::config::ProviderPreset;
    if let Ok(preset) = provider_name.parse::<ProviderPreset>()
        && let Some(ep) = preset.oauth_endpoints()
    {
        return Some(ep);
    }
    crate::config::extension_oauth_endpoints(provider_name)
}

/// Constructs a [`WebAdapter`] with all callbacks wired to `config` and `runtime`.
///
/// Shared by `cmd_start` (when `[channels.web]` is enabled) and `cmd_web_start`.
#[cfg(not(test))]
pub(crate) fn build_web_adapter(config: &Config, runtime: &Arc<AgentRuntime>) -> WebAdapter {
    use crate::config::ProviderPreset;

    let selected_provider = config.agent.provider.name().to_string();
    let selected_model = config.agent.effective_model().to_string();
    let session_loader = Arc::new(MemorySessionLoader(runtime.memory().clone()));

    let runtime_for_web = runtime.clone();
    let config_for_web = config.clone();
    let model_change_cb: channels::web::ModelChangeCallback =
        Arc::new(move |provider_name: String, model: String| {
            runtime_for_web.set_model(model.clone());
            let current = runtime_for_web.active_provider_name();
            if provider_name != current {
                // Refresh credentials before switching to avoid stale OAuth tokens
                if let Ok(preset) = provider_name.parse::<ProviderPreset>() {
                    let handle = tokio::runtime::Handle::current();
                    let fresh_cred = tokio::task::block_in_place(|| {
                        handle.block_on(
                            config_for_web.resolve_credential_for_refreshed(&provider_name),
                        )
                    });
                    if let Some(cred) = fresh_cred {
                        let fresh_provider =
                            build_provider(preset, &cred.api_key, cred.base_url.as_deref(), &model);
                        runtime_for_web.register_provider(&provider_name, fresh_provider);
                    }
                }
                if runtime_for_web.switch_provider(&provider_name).is_err() {
                    info!(
                        "Provider '{}' not yet registered, skipping switch",
                        provider_name
                    );
                }
            }
        });

    let web_models = build_web_model_list();

    let oauth_client_id = config.agent.oauth_client_id.clone();
    let provider_list_cb: channels::web::ProviderListCallback = Arc::new(move || {
        let creds = crate::auth::Credentials::load();
        let registry = crate::config::provider_registry();
        registry
            .iter()
            .map(|entry| channels::web::WebProviderAuthEntry {
                name: entry.name.to_string(),
                display_name: entry.display_name.to_string(),
                auth_mode: entry.auth_mode.as_str().to_string(),
                authenticated: creds.get(entry.name).is_some(),
                supports_runtime: entry.supports_runtime,
            })
            .collect()
    });

    let pending_flows: Arc<
        tokio::sync::Mutex<std::collections::HashMap<String, crate::auth::PendingOAuthCodeDisplay>>,
    > = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let oauth_client_id2 = oauth_client_id.clone();
    let provider_auth_cb: channels::web::ProviderAuthCallback =
        Arc::new(move |intent: channels::web::ProviderAuthIntent| {
            let oauth_cid = oauth_client_id2.clone();
            let pending_flows_clone = pending_flows.clone();
            Box::pin(async move {
                // Case 1: API key provided — store directly
                if let Some(api_key) = &intent.api_key {
                    let cred = crate::auth::ProviderCredential {
                        access_token: api_key.clone(),
                        endpoint: intent.endpoint.clone(),
                        refresh_token: None,
                        expires_at: None,
                        id_token: None,
                    };
                    let mut creds = crate::auth::Credentials::load();
                    creds.set(intent.provider.clone(), cred);
                    creds
                        .save()
                        .map_err(|e| format!("Failed to save credentials: {e}"))?;
                    return Ok(channels::web::ProviderAuthResult::Completed {
                        message: format!("API key saved for {}", intent.provider),
                    });
                }

                // Case 2: Auth code provided — complete code-display flow
                if let Some(auth_code) = &intent.auth_code {
                    let pending = pending_flows_clone
                        .lock()
                        .await
                        .remove(&intent.provider)
                        .ok_or_else(|| format!("No pending OAuth flow for {}", intent.provider))?;
                    // TTL check — reject flows older than 10 minutes
                    if pending.created_at.elapsed() > std::time::Duration::from_secs(600) {
                        return Err(
                            "OAuth flow expired (10 minute timeout). Please try again.".to_string()
                        );
                    }
                    let cred = crate::auth::complete_code_display_flow(&pending, auth_code)
                        .await
                        .map_err(|e| format!("Code exchange failed: {e}"))?;
                    let mut creds = crate::auth::Credentials::load();
                    creds.set(intent.provider.clone(), cred);
                    creds
                        .save()
                        .map_err(|e| format!("Failed to save credentials: {e}"))?;
                    return Ok(channels::web::ProviderAuthResult::Completed {
                        message: format!("Authenticated with {}", intent.provider),
                    });
                }

                // Case 3: OAuth initiation
                let endpoints = resolve_oauth_endpoints(&intent.provider)
                    .ok_or_else(|| format!("No OAuth endpoints for {}", intent.provider))?;
                let client_id = endpoints.effective_client_id(&oauth_cid).ok_or_else(|| {
                    format!(
                        "No OAuth client ID for {}. Set openpista_OAUTH_CLIENT_ID.",
                        intent.provider
                    )
                })?;

                if let Some(callback_port) = endpoints.default_callback_port {
                    let cred = crate::auth::login(
                        &intent.provider,
                        &endpoints,
                        client_id,
                        callback_port,
                        120,
                    )
                    .await
                    .map_err(|e| format!("OAuth login failed: {e}"))?;

                    // GitHub Copilot needs additional token exchange
                    let final_cred = if intent.provider == "github-copilot" {
                        crate::auth::exchange_github_copilot_token(&cred.access_token)
                            .await
                            .map_err(|e| format!("GitHub Copilot token exchange failed: {e}"))?
                    } else {
                        cred
                    };

                    let mut creds = crate::auth::Credentials::load();
                    creds.set(intent.provider.clone(), final_cred);
                    creds
                        .save()
                        .map_err(|e| format!("Failed to save credentials: {e}"))?;
                    Ok(channels::web::ProviderAuthResult::Completed {
                        message: format!("Authenticated with {}", intent.provider),
                    })
                } else {
                    // Code-display flow (Anthropic, OpenRouter) — return URL
                    let pending = crate::auth::start_code_display_flow(
                        &intent.provider,
                        &endpoints,
                        client_id,
                    );
                    let url = pending.auth_url.clone();
                    pending_flows_clone
                        .lock()
                        .await
                        .insert(intent.provider.clone(), pending);
                    Ok(channels::web::ProviderAuthResult::OAuthUrl {
                        url,
                        flow_type: "code_display".to_string(),
                    })
                }
            })
        });

    WebAdapter::new(
        config.channels.web.port,
        config.channels.web.token.clone(),
        config.channels.web.cors_origins.clone(),
        config.channels.web.static_dir.clone(),
        config.channels.web.shared_session_id.clone(),
    )
    .with_selected_model(selected_provider, selected_model)
    .with_session_loader(session_loader)
    .with_model_change_callback(model_change_cb)
    .with_model_list(web_models)
    .with_provider_list_callback(provider_list_cb)
    .with_provider_auth_callback(provider_auth_cb)
}

/// Processes agent events from all channels until shutdown or the sender is dropped.
///
/// Shared by `cmd_start` and `cmd_web_start`.
#[cfg(not(test))]
pub(crate) async fn run_web_event_loop(
    mut event_rx: tokio::sync::mpsc::Receiver<ChannelEvent>,
    runtime: Arc<AgentRuntime>,
    skill_loader: Arc<SkillLoader>,
    resp_tx: tokio::sync::mpsc::Sender<AgentResponse>,
    label: &'static str,
) {
    tokio::select! {
        _ = async {
            while let Some(event) = event_rx.recv().await {
                let runtime = runtime.clone();
                let skill_loader = skill_loader.clone();
                let resp_tx = resp_tx.clone();

                tokio::spawn(async move {
                    let skills_ctx = skill_loader.load_context().await;
                    debug!(
                        channel_id = %event.channel_id,
                        session_id = %event.session_id,
                        label,
                        "Processing channel message"
                    );
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(120),
                        runtime.process(
                            &event.channel_id,
                            &event.session_id,
                            &event.user_message,
                            Some(&skills_ctx),
                        ),
                    )
                    .await;
                    let result = match result {
                        Ok(inner) => inner,
                        Err(_elapsed) => {
                            error!(
                                channel_id = %event.channel_id,
                                session_id = %event.session_id,
                                "Agent processing timed out after 120s"
                            );
                            Err(proto::Error::Timeout)
                        }
                    };
                    let resp = crate::build_agent_response(&event, result);
                    if resp.is_error {
                        error!(
                            channel_id = %event.channel_id,
                            session_id = %event.session_id,
                            "Agent returned error: {:.100}", resp.content
                        );
                    } else {
                        debug!(
                            channel_id = %event.channel_id,
                            session_id = %event.session_id,
                            "Agent response sent ({} chars)", resp.content.len()
                        );
                    }
                    let _ = resp_tx.send(resp).await;
                });
            }
        } => {}
        _ = crate::daemon::wait_for_shutdown() => {
            info!("Shutdown signal received");
        }
    }
}

#[cfg(not(test))]
pub(crate) fn spawn_web_session_sync_task(memory: Arc<dyn Memory>, web_adapter: WebAdapter) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            match memory.list_sessions_with_preview().await {
                Ok(sessions) => {
                    let snapshot: Vec<WebSessionEntry> = sessions
                        .into_iter()
                        .map(|(id, channel_id, updated_at, preview)| WebSessionEntry {
                            id: id.as_str().to_string(),
                            channel_id,
                            updated_at: updated_at.to_rfc3339(),
                            preview,
                        })
                        .collect();
                    debug!(count = snapshot.len(), "Updated web session cache");
                    web_adapter.set_sessions(snapshot);
                }
                Err(e) => {
                    debug!("Failed to refresh web session cache: {e}");
                }
            }
        }
    });
}

#[cfg(not(test))]
/// Configures web adapter settings and installs static web assets.
pub(crate) async fn cmd_web_setup(
    mut config: Config,
    options: WebSetupOptions,
) -> anyhow::Result<()> {
    let WebSetupOptions {
        token,
        regenerate_token,
        yes,
        port,
        cors_origins,
        static_dir,
        shared_session_id,
        enable,
        disable,
    } = options;

    let mut generated_token: Option<String> = None;
    if let Some(token) = token {
        config.channels.web.token = token;
    } else if regenerate_token || config.channels.web.token.is_empty() {
        let new_token = generate_web_setup_token();
        config.channels.web.token = new_token.clone();
        generated_token = Some(new_token);
    } else {
        let should_regenerate = if yes {
            true
        } else if is_interactive_terminal() {
            prompt_yes_no("Web token already exists. Regenerate?", false)?
        } else {
            false
        };

        if should_regenerate {
            let new_token = generate_web_setup_token();
            config.channels.web.token = new_token.clone();
            generated_token = Some(new_token);
        }
    }

    if let Some(port) = port {
        if port == 0 {
            anyhow::bail!("--port must be between 1 and 65535");
        }
        config.channels.web.port = port;
    }
    if let Some(cors_origins) = cors_origins {
        config.channels.web.cors_origins = cors_origins;
    }
    if let Some(static_dir) = static_dir {
        config.channels.web.static_dir = static_dir;
    }
    if let Some(shared_session_id) = shared_session_id {
        config.channels.web.shared_session_id = shared_session_id.trim().to_string();
    }
    if enable {
        config.channels.web.enabled = true;
    }
    if disable {
        config.channels.web.enabled = false;
    }

    let source_dir = find_web_static_source_dir().ok_or_else(|| {
        anyhow::anyhow!(
            "Web static source not found. Expected `crates/channels/static` in current workspace."
        )
    })?;
    let target_dir = expand_tilde_path(&config.channels.web.static_dir);
    copy_directory_recursive(&source_dir, &target_dir)?;

    config
        .save_web_section()
        .map_err(|e| anyhow::anyhow!("failed to save web config: {e}"))?;

    println!("Web setup completed.");
    println!("  enabled: {}", config.channels.web.enabled);
    println!(
        "  token set: {}",
        if config.channels.web.token.is_empty() {
            "no"
        } else {
            "yes"
        }
    );
    println!("  port: {}", config.channels.web.port);
    println!("  cors_origins: {}", config.channels.web.cors_origins);
    println!("  static_dir: {}", config.channels.web.static_dir);
    println!(
        "  shared_session_id: {}",
        if config.channels.web.shared_session_id.trim().is_empty() {
            "(empty -> web:<client_id>)"
        } else {
            config.channels.web.shared_session_id.as_str()
        }
    );
    if let Some(generated_token) = generated_token {
        println!("  generated_token: {generated_token}");
        println!("  note: store this token securely; it will not be shown again.");
    }
    println!("  installed_from: {}", source_dir.display());
    println!("  installed_to: {}", target_dir.display());
    Ok(())
}

#[cfg(not(test))]
/// Starts daemon mode with only the web channel adapter enabled.
pub(crate) async fn cmd_web_start(config: Config) -> anyhow::Result<()> {
    info!("Starting openpista web-only daemon");

    if !config.channels.web.enabled {
        warn!("Web adapter is disabled in config; `web start` will start it anyway");
    }
    if config.channels.web.token.is_empty() {
        warn!("Web adapter token is empty; websocket auth is disabled");
    }
    ensure_web_port_available(config.channels.web.port).await?;

    let runtime = build_runtime(&config, Arc::new(AutoApproveHandler)).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));

    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<ChannelEvent>(128);
    let (resp_tx, mut resp_rx) = tokio::sync::mpsc::channel::<AgentResponse>(128);

    let web_adapter = build_web_adapter(&config, &runtime);
    // Wire web approval handler into the runtime so tool calls go through the browser
    runtime.set_approval_handler(web_adapter.approval_handler());
    let web_session_sync_adapter = web_adapter.clone();
    let web_resp_adapter = web_adapter.clone();

    tokio::spawn(async move {
        if let Err(e) = web_adapter.run(event_tx).await {
            error!("Web adapter error: {e}");
        }
    });

    spawn_web_session_sync_task(runtime.memory().clone(), web_session_sync_adapter);

    tokio::spawn(async move {
        while let Some(resp) = resp_rx.recv().await {
            let channel_id = resp.channel_id.clone();
            if crate::should_send_web_response(&channel_id) {
                if let Err(e) = web_resp_adapter.send_response(resp).await {
                    error!("Failed to send Web response: {e}");
                }
                continue;
            }
            warn!(
                "No web response adapter configured for channel: {}",
                channel_id
            );
        }
    });

    let pid_file = crate::daemon::PidFile::new(crate::daemon::PidFile::default_path());
    pid_file.write().await?;
    let port = config.channels.web.port;
    println!("Web server started:");
    println!("  http: http://127.0.0.1:{port}");
    println!("  ws: ws://127.0.0.1:{port}/ws");
    println!("  health: http://127.0.0.1:{port}/health");

    run_web_event_loop(event_rx, runtime, skill_loader, resp_tx, "web").await;

    pid_file.remove().await;
    info!("openpista web-only daemon stopped");
    Ok(())
}

#[cfg(not(test))]
/// Prints web adapter configuration and runtime status.
pub(crate) async fn cmd_web_status(config: Config) -> anyhow::Result<()> {
    let web = &config.channels.web;
    let static_dir = expand_tilde_path(&web.static_dir);

    let pid_path = crate::daemon::PidFile::default_path();
    // Attempt to read the PID file directly; absence is treated as no PID.
    let pid = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok());
    let pid_file_exists = pid.is_some();
    let process_alive = pid.and_then(is_process_alive);

    let health = check_web_health(web.port).await;
    let health_ok = matches!(health, WebHealthStatus::Healthy);

    let overall = if process_alive == Some(true) && health_ok {
        "running"
    } else if pid_file_exists || health_ok {
        "partial"
    } else {
        "stopped"
    };

    println!("Web Adapter Config:");
    println!("  enabled: {}", web.enabled);
    println!(
        "  token set: {}",
        if web.token.is_empty() { "no" } else { "yes" }
    );
    println!("  port: {}", web.port);
    println!("  cors_origins: {}", web.cors_origins);
    println!("  static_dir: {}", web.static_dir);
    println!(
        "  shared_session_id: {}",
        if web.shared_session_id.trim().is_empty() {
            "(empty -> web:<client_id>)"
        } else {
            web.shared_session_id.as_str()
        }
    );
    println!("  static_dir_exists: {}", static_dir.exists());
    println!();
    println!("Web Runtime:");
    println!("  pid_file: {}", pid_path.display());
    println!("  pid_file_exists: {pid_file_exists}");
    match pid {
        Some(pid) => println!("  pid: {}", pid),
        None => println!("  pid: (none)"),
    }
    match process_alive {
        Some(true) => println!("  process_alive: yes"),
        Some(false) => println!("  process_alive: no"),
        None => println!("  process_alive: unknown"),
    }
    println!("  health: {}", health.as_text());
    println!("  overall: {overall}");

    Ok(())
}

#[cfg(not(test))]
enum WebHealthStatus {
    Healthy,
    Http(u16),
    Error(String),
}

#[cfg(not(test))]
impl WebHealthStatus {
    fn as_text(&self) -> String {
        match self {
            Self::Healthy => "ok".to_string(),
            Self::Http(status) => format!("http {status}"),
            Self::Error(err) => format!("error: {err}"),
        }
    }
}

#[cfg(not(test))]
async fn check_web_health(port: u16) -> WebHealthStatus {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1))
        .build()
    {
        Ok(c) => c,
        Err(e) => return WebHealthStatus::Error(e.to_string()),
    };

    match client.get(url).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                WebHealthStatus::Healthy
            } else {
                WebHealthStatus::Http(resp.status().as_u16())
            }
        }
        Err(e) => WebHealthStatus::Error(e.to_string()),
    }
}

#[cfg(not(test))]
pub(crate) async fn ensure_web_port_available(port: u16) -> anyhow::Result<()> {
    let addr = format!("0.0.0.0:{port}");
    match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(e) => anyhow::bail!(
            "port {} is already in use ({}). Stop the current process manually or change port with `openpista web setup --port <PORT>`.",
            port,
            e
        ),
    }
}

#[cfg(all(not(test), unix))]
fn is_process_alive(pid: u32) -> Option<bool> {
    let status = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .ok()?;
    Some(status.success())
}

#[cfg(all(not(test), not(unix)))]
fn is_process_alive(_pid: u32) -> Option<bool> {
    None
}

#[cfg(not(test))]
pub(crate) fn expand_tilde_path(path: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(proto::path::expand_tilde(path))
}

#[cfg(not(test))]
fn find_web_static_source_dir() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    // Prefer Trunk dist output (WASM build) when available.
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("crates/web/dist"));
        candidates.push(cwd.join("crates/channels/static"));
    }
    candidates.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/dist"));
    candidates
        .push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../channels/static"));

    candidates
        .into_iter()
        .find(|path| path.exists() && path.is_dir())
}

#[cfg(not(test))]
fn copy_directory_recursive(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)
        .map_err(|e| anyhow::anyhow!("cannot read source directory {}: {e}", src.display()))?
    {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            copy_directory_recursive(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

#[cfg(not(test))]
fn generate_web_setup_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(not(test))]
pub(crate) fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

#[cfg(not(test))]
pub(crate) fn prompt_yes_no(question: &str, default_yes: bool) -> anyhow::Result<bool> {
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{question} {suffix}: ");
    std::io::stdout().flush()?;

    let mut input = String::new();
    let bytes = std::io::stdin().read_line(&mut input)?;
    if bytes == 0 {
        return Ok(default_yes);
    }
    let answer = input.trim().to_ascii_lowercase();
    if answer.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(answer.as_str(), "y" | "yes"))
}
