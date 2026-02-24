//! CLI entrypoint and subcommand orchestration.

mod auth;
mod auth_picker;
mod config;
mod daemon;
mod model_catalog;
#[cfg(test)]
mod test_support;
mod tui;

use clap::{Parser, Subcommand};
use proto::{AgentResponse, ChannelEvent, ChannelId, SessionId};

#[cfg(not(test))]
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
#[cfg(not(test))]
use rand::{RngCore, rngs::OsRng};
#[cfg(not(test))]
use std::io::{IsTerminal, Write};
#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use crate::auth::is_openai_oauth_credential_for_key;
#[cfg(not(test))]
use crate::auth_picker::{AuthLoginIntent, AuthMethodChoice};
#[cfg(not(test))]
use agent::{
    AgentRuntime, AnthropicProvider, AutoApproveHandler, OpenAiProvider, ResponsesApiProvider,
    SqliteMemory, ToolRegistry,
};
#[cfg(not(test))]
use channels::{
    ChannelAdapter, CliAdapter, SessionLoader, TelegramAdapter, WebAdapter, WebModelEntry,
    WebSessionEntry, WhatsAppAdapter,
};
use config::Config;
#[cfg(not(test))]
use skills::SkillLoader;
#[cfg(not(test))]
use tools::{
    BashTool, BrowserClickTool, BrowserScreenshotTool, BrowserTool, BrowserTypeTool, ContainerTool,
    ScreenTool,
};
#[cfg(not(test))]
use tracing::{debug, error, info, warn};
#[cfg(not(test))]
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Top-level command-line arguments for the openpista application.
#[derive(Parser)]
#[command(name = "openpista")]
#[command(about = "OS Gateway AI Agent", version = "0.1.0")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Enable debug logging to ~/.openpista/debug.log
    #[arg(long, default_value_t = false)]
    debug: bool,

    /// Resume an existing session by its ID (shortcut for `tui -s <id>`)
    #[arg(short = 's', long)]
    session: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// CLI subcommands available in the application.
#[derive(Subcommand)]
enum Commands {
    /// Start the full-screen TUI (default when no subcommand is given)
    Tui {
        /// Resume an existing session by its ID
        #[arg(short = 's', long)]
        session: Option<String>,
    },

    /// Start the daemon (all enabled channels)
    Start,

    /// Run a single command and exit
    Run {
        /// Command or message to send to the agent
        #[arg(short = 'e', long)]
        exec: String,
    },

    /// Browse or test model catalog entries
    Model {
        /// 'list' to show catalog, 'test' to test all, or a model name to test
        #[arg(value_name = "MODEL_OR_COMMAND")]
        model_name: Option<String>,

        /// Message to send for testing
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
    /// Manage provider credentials via OAuth 2.0 PKCE browser login
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },


    /// Manage Web adapter setup/start/status flows
    Web {
        #[command(subcommand)]
        command: WebCommands,
    },

    /// WhatsApp channel management
    Whatsapp {
        #[command(subcommand)]
        command: Option<WhatsAppCommands>,
    },

    /// Manage Telegram channel (status, setup guide, enable)
    Telegram {
        #[command(subcommand)]
        command: TelegramCommands,
    },
}

/// `whatsapp` sub-subcommands.
#[derive(Subcommand)]
enum WhatsAppCommands {
    /// Set up WhatsApp (QR pairing) — same as bare `openpista whatsapp`
    Setup,
    /// Start WhatsApp bridge in foreground mode (bot)
    Start,
    /// Show WhatsApp connection status
    Status,
    /// Send a message to a WhatsApp number
    Send {
        /// Phone number (e.g. 821012345678)
        number: String,
        /// Message text
        message: Vec<String>,
    },
}

/// `auth` sub-subcommands.
/// `telegram` sub-subcommands.
#[derive(Subcommand)]
enum TelegramCommands {
    /// Show current Telegram channel configuration and readiness
    Status,
    /// Save bot token to config.toml (get it from @BotFather in Telegram)
    Setup {
        /// Bot token from @BotFather (e.g. 123456:ABC...)
        #[arg(long)]
        token: Option<String>,
    },
    /// Start the Telegram bot server (reads token from config or TELEGRAM_BOT_TOKEN env)
    Start {
        /// Override bot token for this session only (not saved)
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Authenticate with a provider via browser-based OAuth PKCE flow
    Login {
        /// Provider to authenticate with
        #[arg(short, long)]
        provider: Option<String>,

        /// API key value to persist for API-key or fallback auth modes
        #[arg(long)]
        api_key: Option<String>,

        /// Endpoint value for endpoint+key providers (azure-openai, custom, etc.)
        #[arg(long)]
        endpoint: Option<String>,

        /// Local port for the OAuth callback server
        #[arg(long, default_value_t = 9009)]
        port: u16,

        /// Seconds to wait for the browser authorization before timing out
        #[arg(long, default_value_t = 120)]
        timeout: u64,

        /// Skip interactive picker and require flags/env inputs only
        #[arg(long, default_value_t = false)]
        non_interactive: bool,
    },

    /// Remove stored credentials for a provider
    Logout {
        /// Provider to log out from (openai, openrouter)
        #[arg(short, long, default_value = "openai")]
        provider: String,
    },

    /// Show current authentication status for all stored providers
    Status,
}

/// `web` subcommands.
#[derive(Subcommand)]
enum WebCommands {
    /// Configure web adapter and install static assets to static_dir
    Setup {
        /// WebSocket auth token to persist
        #[arg(long)]
        token: Option<String>,

        /// Force-generate and persist a new WebSocket auth token
        #[arg(long, default_value_t = false, conflicts_with = "token")]
        regenerate_token: bool,

        /// Auto-confirm token regeneration prompt
        #[arg(short = 'y', long, default_value_t = false)]
        yes: bool,

        /// HTTP/WS listen port
        #[arg(long)]
        port: Option<u16>,

        /// Allowed CORS origins (comma-separated or "*")
        #[arg(long)]
        cors_origins: Option<String>,

        /// Static files directory to serve
        #[arg(long)]
        static_dir: Option<String>,

        /// Shared session id used by web and TUI when `-s` is omitted
        #[arg(long)]
        shared_session_id: Option<String>,

        /// Force-enable the web adapter
        #[arg(long, default_value_t = false, conflicts_with = "disable")]
        enable: bool,

        /// Force-disable the web adapter
        #[arg(long, default_value_t = false, conflicts_with = "enable")]
        disable: bool,
    },

    /// Start daemon in web-only mode
    Start,

    /// Show web adapter configuration and runtime status
    Status,
}

#[cfg(not(test))]
struct WebSetupOptions {
    token: Option<String>,
    regenerate_token: bool,
    yes: bool,
    port: Option<u16>,
    cors_origins: Option<String>,
    static_dir: Option<String>,
    shared_session_id: Option<String>,
    enable: bool,
    disable: bool,
}

#[cfg(not(test))]
#[tokio::main]
/// Program entrypoint.
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Determine effective command (default to Tui if none given)
    let command = cli.command.unwrap_or(Commands::Tui {
        session: cli.session.clone(),
    });
    let is_tui = matches!(command, Commands::Tui { .. });

    // Initialize tracing — suppress console output in TUI mode to avoid corrupting the display.
    // When --debug is passed, write debug-level logs to ~/.openpista/logs/debug.YYYY-MM-DD.log
    // using daily rotation so logs accumulate across sessions.
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    // WorkerGuard must outlive main() so buffered file writes are flushed on exit.
    let _file_guard: Option<tracing_appender::non_blocking::WorkerGuard>;

    let debug_writer = if cli.debug {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let log_dir = std::path::PathBuf::from(home)
            .join(".openpista")
            .join("logs");
        std::fs::create_dir_all(&log_dir).ok();
        let appender = tracing_appender::rolling::daily(&log_dir, "debug.log");
        let (writer, guard) = tracing_appender::non_blocking(appender);
        _file_guard = Some(guard);
        Some(writer)
    } else {
        _file_guard = None;
        None
    };

    match (is_tui, debug_writer) {
        (true, Some(writer)) => {
            let console = fmt::layer()
                .with_writer(std::io::sink)
                .with_target(false)
                .with_filter(console_filter);
            let file = fmt::layer()
                .with_writer(writer)
                .with_target(true)
                .with_ansi(false)
                .with_filter(EnvFilter::new(
                    "debug,hyper_util=info,rustls=info,reqwest=info,sqlx=info",
                ));
            tracing_subscriber::registry()
                .with(console)
                .with(file)
                .init();
        }
        (true, None) => {
            fmt()
                .with_env_filter(console_filter)
                .with_writer(std::io::sink)
                .with_target(false)
                .init();
        }
        (false, Some(writer)) => {
            let console = fmt::layer().with_target(false).with_filter(console_filter);
            let file = fmt::layer()
                .with_writer(writer)
                .with_target(true)
                .with_ansi(false)
                .with_filter(EnvFilter::new(
                    "debug,hyper_util=info,rustls=info,reqwest=info,sqlx=info",
                ));
            tracing_subscriber::registry()
                .with(console)
                .with(file)
                .init();
        }
        (false, None) => {
            fmt()
                .with_env_filter(console_filter)
                .with_target(false)
                .init();
        }
    }

    // Emit session-start marker when --debug is active so each run is easily identifiable.
    if cli.debug {
        let cmd_label = match &command {
            Commands::Tui { .. } => "tui",
            Commands::Start => "start",
            Commands::Run { .. } => "run",
            Commands::Model { .. } => "model",
            Commands::Auth { .. } => "auth",
            Commands::Web { .. } => "web",
            Commands::Whatsapp { .. } => "whatsapp",
            Commands::Telegram { .. } => "telegram",
        };
        info!(
            version = env!("CARGO_PKG_VERSION"),
            command = cmd_label,
            log_level = %cli.log_level,
            "========== openpista session start =========="
        );
    }

    // Load config
    let config = Config::load(cli.config.as_deref()).unwrap_or_else(|e| {
        warn!("Failed to load config ({e}), using defaults");
        Config::default()
    });

    match command {
        Commands::Tui { session } => cmd_tui(config, session.or(cli.session)).await,
        Commands::Start => cmd_start(config).await,
        Commands::Run { exec } => cmd_run(config, exec).await,
        Commands::Model {
            model_name,
            message,
        } => match model_name.as_deref() {
            None | Some("select") => cmd_model_select(config).await,
            Some("list") => cmd_models(config).await,
            Some("test") => {
                let msg = message.unwrap_or_else(|| "Hello! Please respond briefly.".to_string());
                cmd_model_test_all(config, msg).await
            }
            Some(name) => {
                let msg = message.unwrap_or_else(|| "Hello! Please respond briefly.".to_string());
                cmd_model_test(config, name.to_string(), msg).await
            }
        },
        Commands::Auth { command } => match command {
            AuthCommands::Login {
                provider,
                api_key,
                endpoint,
                port,
                timeout,
                non_interactive,
            } => {
                cmd_auth_login(
                    config,
                    provider,
                    api_key,
                    endpoint,
                    port,
                    timeout,
                    non_interactive,
                )
                .await
            }
            AuthCommands::Logout { provider } => cmd_auth_logout(provider),
            AuthCommands::Status => cmd_auth_status(),
        },
        Commands::Web { command } => match command {
            WebCommands::Setup {
                token,
                regenerate_token,
                yes,
                port,
                cors_origins,
                static_dir,
                shared_session_id,
                enable,
                disable,
            } => {
                let options = WebSetupOptions {
                    token,
                    regenerate_token,
                    yes,
                    port,
                    cors_origins,
                    static_dir,
                    shared_session_id,
                    enable,
                    disable,
                };
                cmd_web_setup(config, options).await
            }
            WebCommands::Start => cmd_web_start(config).await,
            WebCommands::Status => cmd_web_status(config).await,
        },
        Commands::Whatsapp { command } => match command {
            None | Some(WhatsAppCommands::Setup) => cmd_whatsapp(config).await,
            Some(WhatsAppCommands::Start) => cmd_whatsapp_start(config).await,
            Some(WhatsAppCommands::Status) => cmd_whatsapp_status(config).await,
            Some(WhatsAppCommands::Send { number, message }) => {
                cmd_whatsapp_send(config, number, message.join(" ")).await
            }
        },
        Commands::Telegram { command } => match command {
            TelegramCommands::Status => cmd_telegram_status(&config),
            TelegramCommands::Setup { token } => cmd_telegram_setup(config, token).await,
            TelegramCommands::Start { token } => cmd_telegram_start(config, token).await,
        },
    }
}

#[cfg(not(test))]
/// Starts the full-screen TUI for interactive agent sessions.
async fn cmd_tui(config: Config, session: Option<String>) -> anyhow::Result<()> {
    let (tui_approval_handler, approval_rx) = tui::approval::TuiApprovalHandler::new();
    let runtime = build_runtime(&config, Arc::new(tui_approval_handler)).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));
    let channel_id = ChannelId::new("cli", "tui");
    let session_id = resolve_tui_session_id(&config, session);
    let tui_state = config::TuiState::load();
    let model_name = if config.agent.model.is_empty() && !tui_state.last_model.is_empty() {
        tui_state.last_model.clone()
    } else {
        config.agent.effective_model().to_string()
    };

    tui::run_tui(
        runtime,
        skill_loader,
        channel_id,
        session_id.clone(),
        model_name,
        config.clone(),
        approval_rx,
    )
    .await?;

    print_goodbye_banner(&session_id, config.agent.effective_model());
    Ok(())
}

fn resolve_tui_session_id(config: &Config, explicit_session: Option<String>) -> SessionId {
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
fn build_provider(
    preset: config::ProviderPreset,
    api_key: &str,
    base_url: Option<&str>,
    model: &str,
) -> Arc<dyn agent::LlmProvider> {
    match preset {
        config::ProviderPreset::Anthropic => {
            if let Some(base_url) = base_url {
                Arc::new(AnthropicProvider::with_base_url(api_key, base_url))
            } else {
                Arc::new(AnthropicProvider::new(api_key))
            }
        }
        _ => {
            // Detect OAuth-based credential → use Responses API for subscription access
            let use_responses_api = preset == config::ProviderPreset::OpenAi
                && is_openai_oauth_credential_for_key(api_key);
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
async fn build_runtime(
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
    let memory = Arc::new(memory);

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
    for preset in config::ProviderPreset::all() {
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

#[cfg(not(test))]
/// Starts daemon mode with enabled channel adapters.
async fn cmd_start(config: Config) -> anyhow::Result<()> {
    info!("Starting openpista daemon");

    let runtime = build_runtime(&config, Arc::new(AutoApproveHandler)).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));

    // In-process event bus
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<ChannelEvent>(128);
    let (resp_tx, mut resp_rx) = tokio::sync::mpsc::channel::<AgentResponse>(128);

    // Channel adapters
    let mut telegram_resp_adapter: Option<TelegramAdapter> = None;
    if config.channels.telegram.enabled {
        let token = config.channels.telegram.token.clone();
        if token.is_empty() {
            warn!("Telegram enabled but no token configured");
        } else {
            let tx = event_tx.clone();
            let adapter = TelegramAdapter::new(token.clone());
            telegram_resp_adapter = Some(TelegramAdapter::new(token));

            tokio::spawn(async move {
                if let Err(e) = adapter.run(tx).await {
                    error!("Telegram adapter error: {e}");
                }
            });
        }
    }

    // CLI adapter (if enabled)
    let mut cli_resp_adapter: Option<CliAdapter> = None;
    if config.channels.cli.enabled {
        let tx = event_tx.clone();
        let session_id = SessionId::new();
        let cli_adapter = CliAdapter::with_session(session_id.clone());
        cli_resp_adapter = Some(CliAdapter::with_session(session_id));
        tokio::spawn(async move {
            if let Err(e) = cli_adapter.run(tx).await {
                error!("CLI adapter error: {e}");
            }
        });
    }

    // WhatsApp adapter
    let mut whatsapp_resp_adapter: Option<WhatsAppAdapter> = None;
    if config.channels.whatsapp.enabled {
        let wa_config = channels::whatsapp::WhatsAppAdapterConfig {
            session_dir: config.channels.whatsapp.session_dir.clone(),
            bridge_path: config.channels.whatsapp.bridge_path.clone(),
        };
        let tx = event_tx.clone();
        let (qr_tx, _qr_rx) = tokio::sync::mpsc::channel::<String>(8);
        let adapter = WhatsAppAdapter::new(wa_config.clone(), resp_tx.clone(), qr_tx.clone());
        whatsapp_resp_adapter = Some(WhatsAppAdapter::new(wa_config, resp_tx.clone(), qr_tx));
        tokio::spawn(async move {
            if let Err(e) = adapter.run(tx).await {
                error!("WhatsApp adapter error: {e}");
            }
        });
    }

    let mut web_resp_adapter: Option<WebAdapter> = None;
    if config.channels.web.enabled {
        if config.channels.web.token.is_empty() {
            warn!("Web adapter enabled with empty token; websocket auth is disabled");
        }

        let tx = event_tx.clone();
        let adapter = build_web_adapter(&config, &runtime);
        let session_sync_adapter = adapter.clone();
        web_resp_adapter = Some(adapter.clone());
        // Wire web approval handler into the runtime
        runtime.set_approval_handler(adapter.approval_handler());

        tokio::spawn(async move {
            if let Err(e) = adapter.run(tx).await {
                error!("Web adapter error: {e}");
            }
        });

        spawn_web_session_sync_task(runtime.memory().clone(), session_sync_adapter);
    }

    // Response forwarder (always consume `resp_rx` to avoid dropped/backed-up responses)
    tokio::spawn(async move {
        while let Some(resp) = resp_rx.recv().await {
            let channel_id = resp.channel_id.clone();

            if should_send_cli_response(&channel_id) {
                if let Some(adapter) = &cli_resp_adapter {
                    if let Err(e) = adapter.send_response(resp).await {
                        error!("Failed to send CLI response: {e}");
                    }
                } else {
                    warn!("CLI response dropped because CLI channel is disabled");
                }
                continue;
            }

            if should_send_telegram_response(&channel_id) {
                if let Some(adapter) = &telegram_resp_adapter {
                    if let Err(e) = adapter.send_response(resp).await {
                        error!("Failed to send Telegram response: {e}");
                    }
                } else {
                    warn!("Telegram response dropped because Telegram channel is disabled");
                }
                continue;
            }

            if should_send_whatsapp_response(&channel_id) {
                if let Some(adapter) = &whatsapp_resp_adapter {
                    if let Err(e) = adapter.send_response(resp).await {
                        error!("Failed to send WhatsApp response: {e}");
                    }
                } else {
                    warn!("WhatsApp response dropped because WhatsApp channel is disabled");
                }
                continue;
            }


            if should_send_web_response(&channel_id) {
                if let Some(adapter) = &web_resp_adapter {
                    if let Err(e) = adapter.send_response(resp).await {
                        error!("Failed to send Web response: {e}");
                    }
                } else {
                    warn!("Web response dropped because Web channel is disabled");
                }
                continue;
            }

            warn!("No response adapter configured for channel: {}", channel_id);
        }
    });

    // PID file
    let pid_file = daemon::PidFile::new(daemon::PidFile::default_path());
    pid_file.write().await?;

    // Main event processing loop
    run_web_event_loop(event_rx, runtime, skill_loader, resp_tx, "daemon").await;

    pid_file.remove().await;
    info!("openpista stopped");
    Ok(())
}

/// Bridges `SqliteMemory` to the `SessionLoader` trait expected by `WebAdapter`.
#[cfg(not(test))]
struct MemorySessionLoader(Arc<SqliteMemory>);

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
fn build_web_model_list() -> Vec<WebModelEntry> {
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
fn resolve_oauth_endpoints(provider_name: &str) -> Option<config::OAuthEndpoints> {
    if let Ok(preset) = provider_name.parse::<config::ProviderPreset>()
        && let Some(ep) = preset.oauth_endpoints()
    {
        return Some(ep);
    }
    config::extension_oauth_endpoints(provider_name)
}

/// Constructs a [`WebAdapter`] with all callbacks wired to `config` and `runtime`.
///
/// Shared by `cmd_start` (when `[channels.web]` is enabled) and `cmd_web_start`.
#[cfg(not(test))]
fn build_web_adapter(config: &Config, runtime: &Arc<AgentRuntime>) -> WebAdapter {
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
                if let Ok(preset) = provider_name.parse::<config::ProviderPreset>() {
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
async fn run_web_event_loop(
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
                    let resp = build_agent_response(&event, result);
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
        _ = daemon::wait_for_shutdown() => {
            info!("Shutdown signal received");
        }
    }
}

#[cfg(not(test))]
fn spawn_web_session_sync_task(memory: Arc<SqliteMemory>, web_adapter: WebAdapter) {
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
async fn cmd_web_setup(mut config: Config, options: WebSetupOptions) -> anyhow::Result<()> {
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

// ─── Telegram CLI commands ───────────────────────────────────────────────────

/// Validates that a Telegram bot token matches the expected `NUMBERS:STRING` format.
fn is_valid_telegram_token(token: &str) -> bool {
    let mut parts = token.splitn(2, ':');
    let numeric = parts
        .next()
        .map(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty());
    let rest = parts.next().map(|p| !p.is_empty());
    matches!((numeric, rest), (Some(true), Some(true)))
}

#[cfg(not(test))]
/// `openpista telegram status` — prints current Telegram channel configuration.
fn cmd_telegram_status(config: &Config) -> anyhow::Result<()> {
    println!("Telegram Status");
    println!("===============");
    println!();
    let enabled = config.channels.telegram.enabled;
    let token_set = !config.channels.telegram.token.is_empty();
    println!(
        "  Token   : {}",
        if token_set { "(set)" } else { "(not set)" }
    );
    println!();
    match (enabled, token_set) {
        (true, true) => {
            println!("  Status  : Ready — run `openpista telegram start` to start the bot.");
        }
        (false, true) => {
            println!("  Status  : Token saved but channel not enabled.");
            println!("           Run `openpista telegram start` to start the bot.");
        }
        (_, false) => {
            println!("  Status  : Not configured.");
            println!("           Run `openpista telegram setup --token YOUR_TOKEN` first.");
        }
    }
    Ok(())
}

#[cfg(not(test))]
/// `openpista telegram setup --token TOKEN` — validates and saves bot token to config.toml.
async fn cmd_telegram_setup(mut config: Config, token: Option<String>) -> anyhow::Result<()> {
    println!("Telegram Setup");
    println!("==============");
    println!();

    // Resolve token: flag > env var
    let resolved = token.or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok());

    let token = match resolved {
        Some(t) => t,
        None => {
            eprintln!("Error: no bot token provided.");
            eprintln!();
            eprintln!("How to get a token:");
            eprintln!("  1. Open Telegram and search for @BotFather");
            eprintln!("  2. Send /newbot and follow the prompts");
            eprintln!("  3. Copy the token (format: 123456:ABC...)");
            eprintln!();
            eprintln!("Then run:");
            eprintln!("  openpista telegram setup --token YOUR_TOKEN");
            anyhow::bail!("missing Telegram bot token");
        }
    };

    if !is_valid_telegram_token(&token) {
        anyhow::bail!(
            "Invalid token format '{}'. Expected NUMBERS:STRING (e.g. 123456:ABC...)",
            token
        );
    }

    // Verify the token works by calling the Telegram getMe API
    print!("Verifying token with Telegram API... ");
    let url = format!("https://api.telegram.org/bot{token}/getMe");
    let resp = reqwest::get(&url).await?;
    if resp.status().is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let username = body["result"]["username"].as_str().unwrap_or("unknown");
        println!("OK");
        println!("  Bot name: @{username}");
        println!();
    } else {
        println!("FAILED");
        anyhow::bail!(
            "Token verification failed (HTTP {}). Check the token and try again.",
            resp.status()
        );
    }

    // Save to config
    config.channels.telegram.token = token;
    config.channels.telegram.enabled = true;
    config
        .save()
        .map_err(|e| anyhow::anyhow!("Failed to save config: {e}"))?;
    println!("Telegram channel enabled.");
    println!();
    println!("Run `openpista telegram start` to start the bot server.");
    Ok(())
}

#[cfg(not(test))]
/// `openpista telegram start` — starts the Telegram bot server and agent runtime.
async fn cmd_telegram_start(config: Config, token: Option<String>) -> anyhow::Result<()> {
    use tokio::signal;

    // Resolve token: flag > env var > config
    let resolved = token
        .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
        .or_else(|| {
            if config.channels.telegram.token.is_empty() {
                None
            } else {
                Some(config.channels.telegram.token.clone())
            }
        });

    let token = match resolved {
        Some(t) => t,
        None => {
            eprintln!("Error: no bot token found.");
            eprintln!();
            eprintln!("Run `openpista telegram setup --token YOUR_TOKEN` first,");
            eprintln!("or set the TELEGRAM_BOT_TOKEN environment variable.");
            anyhow::bail!("missing Telegram bot token");
        }
    };

    if !is_valid_telegram_token(&token) {
        anyhow::bail!(
            "Invalid token format '{}'. Expected NUMBERS:STRING (e.g. 123456:ABC...)",
            token
        );
    }

    println!("Telegram Bot Server");
    println!("===================");
    println!();
    println!("Starting agent runtime...");

    let runtime = build_runtime(&config).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));

    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<ChannelEvent>(128);
    let (resp_tx, mut resp_rx) = tokio::sync::mpsc::channel::<AgentResponse>(128);

    // Spawn Telegram adapter (receives messages from users)
    let tg_adapter = TelegramAdapter::new(token.clone());
    let tg_resp_adapter = TelegramAdapter::new(token.clone());
    let event_tx_tg = event_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = tg_adapter.run(event_tx_tg).await {
            error!("Telegram adapter error: {e}");
        }
    });

    println!("Bot is running. Press Ctrl+C to stop.");
    println!();

    // Agent processing loop: event → LLM → response
    let runtime_loop = Arc::clone(&runtime);
    let resp_tx_loop = resp_tx.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let rt = Arc::clone(&runtime_loop);
            let tx = resp_tx_loop.clone();
            let sl = Arc::clone(&skill_loader);
            tokio::spawn(async move {
                let skills_ctx = sl.load_context().await;
                let result = rt
                    .process(
                        &event.channel_id,
                        &event.session_id,
                        &event.user_message,
                        Some(&skills_ctx),
                    )
                    .await;
                let resp = build_agent_response(&event, result);
                let _ = tx.send(resp).await;
            });
        }
    });

    // Response dispatch: send LLM reply back to user
    tokio::spawn(async move {
        while let Some(resp) = resp_rx.recv().await {
            if should_send_telegram_response(&resp.channel_id)
                && let Err(e) = tg_resp_adapter.send_response(resp).await
            {
                error!("Failed to send Telegram response: {e}");
            }
        }
    });

    // Block until Ctrl+C
    signal::ctrl_c().await?;
    println!();
    println!("Shutting down Telegram bot.");
    Ok(())
}

#[cfg(not(test))]
/// Starts daemon mode with only the web channel adapter enabled.
async fn cmd_web_start(config: Config) -> anyhow::Result<()> {
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
            if should_send_web_response(&channel_id) {
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

    let pid_file = daemon::PidFile::new(daemon::PidFile::default_path());
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
async fn cmd_web_status(config: Config) -> anyhow::Result<()> {
    let web = &config.channels.web;
    let static_dir = expand_tilde_path(&web.static_dir);

    let pid_path = daemon::PidFile::default_path();
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
async fn ensure_web_port_available(port: u16) -> anyhow::Result<()> {
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
fn expand_tilde_path(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(format!("{home}{rest}"))
    } else {
        std::path::PathBuf::from(path)
    }
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
fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

#[cfg(not(test))]
fn prompt_yes_no(question: &str, default_yes: bool) -> anyhow::Result<bool> {
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

#[cfg(not(test))]
/// Executes one command against the agent and exits.
async fn cmd_run(config: Config, exec: String) -> anyhow::Result<()> {
    let runtime = build_runtime(&config, Arc::new(AutoApproveHandler)).await?;
    let skill_loader = SkillLoader::new(&config.skills.workspace);
    let skills_ctx = skill_loader.load_context().await;

    let channel_id = ChannelId::new("cli", "run");
    let session_id = SessionId::new();

    println!("{}", format_run_header(&exec));

    let result = runtime
        .process(&channel_id, &session_id, &exec, Some(&skills_ctx))
        .await;

    match result {
        Ok(text) => {
            println!("{text}");
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }

    Ok(())
}

#[cfg(not(test))]
/// Runs the interactive terminal model picker and returns the selection.
/// Uses an alternate screen with RAII cleanup and returns the result so
/// logging/printing happens after terminal restoration.
fn run_model_picker(
    entries: &[model_catalog::ModelCatalogEntry],
    current_model: &str,
    current_provider: &str,
) -> anyhow::Result<Option<model_catalog::ModelCatalogEntry>> {
    use crossterm::{
        cursor::{Hide, MoveTo, Show},
        event::{Event, KeyCode, KeyEventKind, KeyModifiers, read},
        execute,
        terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode},
    };
    use std::io::{Write, stdout};

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen, Show);
        }
    }

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, Hide)?;
    let _guard = Guard;

    let mut query = String::new();
    let mut cursor: usize = 0;

    loop {
        let query_lc = query.to_ascii_lowercase();
        let visible: Vec<&model_catalog::ModelCatalogEntry> = entries
            .iter()
            .filter(|entry| {
                query_lc.is_empty()
                    || entry.id.to_ascii_lowercase().contains(&query_lc)
                    || entry.provider.to_ascii_lowercase().contains(&query_lc)
            })
            .collect();

        cursor = cursor.min(visible.len().saturating_sub(1));

        let mut out = stdout();
        execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;

        let mut lines: Vec<String> = Vec::new();
        lines.push("Select Model".to_string());
        lines.push(String::new());
        lines.push(format!("Search: {query}"));
        lines.push(format!(
            "Current: {} [{}]",
            if current_model.is_empty() {
                "(none)"
            } else {
                current_model
            },
            current_provider
        ));
        lines.push(String::new());

        if visible.is_empty() {
            lines.push(format!("No matches for '{query}'."));
        } else {
            let term_height = crossterm::terminal::size()
                .map(|(_, h)| h as usize)
                .unwrap_or(24);
            let max_visible = term_height.saturating_sub(9);
            let scroll_start = if cursor >= max_visible {
                cursor - max_visible + 1
            } else {
                0
            };
            let scroll_end = (scroll_start + max_visible).min(visible.len());

            for (idx, entry) in visible
                .iter()
                .enumerate()
                .skip(scroll_start)
                .take(scroll_end - scroll_start)
            {
                let marker = if idx == cursor { ">" } else { " " };
                let rec = if entry.recommended_for_coding {
                    "*"
                } else {
                    " "
                };
                let current_tag = if entry.id == current_model && entry.provider == current_provider
                {
                    " (current)"
                } else {
                    ""
                };

                lines.push(format!(
                    "{marker} {rec} {:<30} [{}]{current_tag}",
                    entry.id, entry.provider
                ));
            }
            if scroll_end < visible.len() {
                lines.push(format!("  ... and {} more", visible.len() - scroll_end));
            }
        }

        lines.push(String::new());
        lines.push(format!(
            "{} model(s) | Up/Down move | Enter select | Type search | Esc cancel",
            visible.len()
        ));

        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);
        for line in &lines {
            let display: String = line.chars().take(width).collect();
            out.write_all(display.as_bytes())?;
            out.write_all(b"\r\n")?;
        }
        out.flush()?;

        let event = read()?;
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
                return Ok(None);
            }
            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                cursor = cursor.saturating_sub(1);
            }
            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                if !visible.is_empty() {
                    cursor = (cursor + 1).min(visible.len().saturating_sub(1));
                }
            }
            (_, KeyCode::Backspace) => {
                query.pop();
                cursor = 0;
            }
            (_, KeyCode::Enter) => {
                if visible.is_empty() {
                    continue;
                }
                return Ok(Some(visible[cursor].clone()));
            }
            (_, KeyCode::Char(ch)) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                query.push(ch);
                cursor = 0;
            }
            _ => {}
        }
    }
}

#[cfg(not(test))]
/// Interactive model selector: loads available models, lets user choose, and persists it.
async fn cmd_model_select(mut config: Config) -> anyhow::Result<()> {
    println!("Loading model catalog...");
    let providers = collect_providers_for_test(&config).await;
    let catalog = model_catalog::load_catalog_multi(&providers).await;

    let mut entries: Vec<model_catalog::ModelCatalogEntry> = catalog
        .entries
        .into_iter()
        .filter(|entry| entry.available)
        .collect();
    entries.sort_by(|a, b| {
        b.recommended_for_coding
            .cmp(&a.recommended_for_coding)
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.id.cmp(&b.id))
    });

    if entries.is_empty() {
        anyhow::bail!(
            "No models available. Check your provider credentials with `openpista auth status`."
        );
    }

    let current_model = config.agent.effective_model().to_string();
    let current_provider = config.agent.provider.name().to_string();
    let selected = run_model_picker(&entries, &current_model, &current_provider)?;

    let Some(selected) = selected else {
        println!("Model selection cancelled.");
        return Ok(());
    };

    if let Ok(preset) = selected.provider.parse::<config::ProviderPreset>() {
        config.agent.provider = preset;
    }
    config.agent.model = selected.id.clone();

    if let Err(e) = config.save() {
        eprintln!("Warning: failed to save config: {e}");
    } else {
        println!("Model set: {} [{}]", selected.id, selected.provider);
        println!("Saved to ~/.openpista/config.toml");
    }

    let tui_state = config::TuiState {
        last_model: selected.id,
        last_provider: selected.provider,
    };
    let _ = tui_state.save();

    Ok(())
}

#[cfg(not(test))]
async fn cmd_models(config: Config) -> anyhow::Result<()> {
    let providers = collect_providers_for_test(&config).await;
    let catalog = model_catalog::load_catalog_multi(&providers).await;
    let summary = model_catalog::model_summary(&catalog.entries, "", false);
    let sections = model_catalog::model_sections(&catalog.entries, "", false);
    let provider_names: Vec<&str> = providers.iter().map(|(n, _, _)| n.as_str()).collect();
    println!(
        "model | providers:{} | total:{} | matched:{} | recommended:{} | available:{}",
        provider_names.join(","),
        summary.total,
        summary.matched,
        summary.recommended,
        summary.available
    );
    for status in &catalog.sync_statuses {
        println!("{status}");
    }
    println!();
    print_model_section("Recommended + Available", &sections.recommended_available);
    print_model_section(
        "Recommended + Unavailable",
        &sections.recommended_unavailable,
    );
    Ok(())
}

#[cfg(not(test))]
fn print_model_section(title: &str, entries: &[model_catalog::ModelCatalogEntry]) {
    println!("{title} ({})", entries.len());
    for entry in entries {
        println!(
            "- {}  [provider:{}]  [status:{}]  [available:{}]  [source:{}]",
            entry.id,
            entry.provider,
            entry.status.as_str(),
            if entry.available { "yes" } else { "no" },
            entry.source.as_str()
        );
    }
    println!();
}

#[cfg(not(test))]
async fn collect_providers_for_test(config: &Config) -> Vec<(String, Option<String>, String)> {
    let mut providers = Vec::new();
    for preset in config::ProviderPreset::all() {
        let name = preset.name();
        if let Some(cred) = config.resolve_credential_for_refreshed(name).await {
            providers.push((name.to_string(), cred.base_url, cred.api_key));
        }
    }
    // Ensure the currently configured provider is always included
    let active = config.agent.provider.name().to_string();
    if !providers.iter().any(|(n, _, _)| n == &active) {
        let key = config.resolve_api_key_refreshed().await;
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

#[cfg(not(test))]
async fn cmd_model_test(
    mut config: Config,
    model_name: String,
    message: String,
) -> anyhow::Result<()> {
    // Look up model in catalog to determine provider
    let providers = collect_providers_for_test(&config).await;
    let catalog = model_catalog::load_catalog_multi(&providers).await;
    let entry = catalog.entries.iter().find(|e| e.id == model_name);

    // Override provider if found in catalog
    if let Some(entry) = entry
        && let Ok(preset) = entry.provider.parse::<config::ProviderPreset>()
    {
        config.agent.provider = preset;
    }
    config.agent.model = model_name.clone();

    let runtime = build_runtime(&config, Arc::new(AutoApproveHandler)).await?;
    let channel_id = ChannelId::new("cli", "model-test");
    let session_id = SessionId::new();
    println!(
        "Testing model: {} (provider: {})",
        model_name,
        config.agent.provider.name()
    );
    println!("Message: {message}");
    println!("---");

    let start = std::time::Instant::now();
    let result = runtime
        .process(&channel_id, &session_id, &message, None)
        .await;
    let elapsed = start.elapsed();

    match result {
        Ok(text) => {
            println!("OK ({:.1}s)\n{text}", elapsed.as_secs_f64());
            info!(model = %model_name, elapsed_ms = %elapsed.as_millis(), "Model test passed");
        }
        Err(e) => {
            eprintln!("FAIL ({:.1}s): {e}", elapsed.as_secs_f64());
            error!(model = %model_name, error = %e, "Model test failed");
            std::process::exit(1);
        }
    }
    Ok(())
}

#[cfg(not(test))]
async fn cmd_model_test_all(config: Config, message: String) -> anyhow::Result<()> {
    let providers = collect_providers_for_test(&config).await;
    let catalog = model_catalog::load_catalog_multi(&providers).await;

    // Filter to recommended + available models
    let test_models: Vec<_> = catalog
        .entries
        .iter()
        .filter(|e| e.recommended_for_coding && e.available)
        .collect();

    if test_models.is_empty() {
        println!("No recommended & available models found. Run `openpista auth login` first.");
        return Ok(());
    }

    println!("Testing all available models with: \"{message}\"\n");

    let mut passed = 0u32;
    let mut failed = 0u32;
    let total = test_models.len();

    for entry in &test_models {
        let mut test_config = config.clone();
        if let Ok(preset) = entry.provider.parse::<config::ProviderPreset>() {
            test_config.agent.provider = preset;
        }
        test_config.agent.model = entry.id.clone();
        let runtime = match build_runtime(&test_config, Arc::new(AutoApproveHandler)).await {
            Ok(rt) => rt,
            Err(e) => {
                println!("  [{}] {:<24} FAIL (setup): {e}", entry.provider, entry.id);
                failed += 1;
                continue;
            }
        };

        let channel_id = ChannelId::new("cli", "model-test");
        let session_id = SessionId::new();

        let start = std::time::Instant::now();
        let result = runtime
            .process(&channel_id, &session_id, &message, None)
            .await;
        let elapsed = start.elapsed();

        match result {
            Ok(text) => {
                let preview: String = text.chars().take(50).collect();
                let preview = preview.replace('\n', " ");
                println!(
                    "  [{}] {:<24} OK ({:.1}s) \u{2014} \"{}{}\"",
                    entry.provider,
                    entry.id,
                    elapsed.as_secs_f64(),
                    preview,
                    if text.len() > 50 { "..." } else { "" }
                );
                info!(model = %entry.id, provider = %entry.provider, elapsed_ms = %elapsed.as_millis(), "Model test passed");
                passed += 1;
            }
            Err(e) => {
                println!(
                    "  [{}] {:<24} FAIL ({:.1}s) \u{2014} {e}",
                    entry.provider,
                    entry.id,
                    elapsed.as_secs_f64()
                );
                error!(model = %entry.id, provider = %entry.provider, error = %e, "Model test failed");
                failed += 1;
            }
        }
    }

    println!("\nResults: {passed} passed, {failed} failed out of {total} models");

    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
#[cfg(not(test))]
async fn cmd_auth_login(
    config: Config,
    provider: Option<String>,
    api_key: Option<String>,
    endpoint: Option<String>,
    port: u16,
    timeout: u64,
    non_interactive: bool,
) -> anyhow::Result<()> {
    use crate::config::{LoginAuthMode, provider_registry_entry_ci, provider_registry_names};

    let lookup_env = |env_name: &str| -> Option<String> {
        if env_name.is_empty() {
            None
        } else {
            std::env::var(env_name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        }
    };

    let intent = if non_interactive {
        let provider = provider
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if provider.is_empty() {
            anyhow::bail!(
                "--non-interactive requires --provider <name>. Available providers: {}",
                provider_registry_names()
            );
        }
        let entry = provider_registry_entry_ci(&provider).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown provider '{provider}'. Available providers: {}",
                provider_registry_names()
            )
        })?;

        let resolved_api_key = api_key
            .filter(|value| !value.trim().is_empty())
            .or_else(|| lookup_env(entry.api_key_env))
            .or_else(|| lookup_env("openpista_API_KEY"));

        let resolved_endpoint = endpoint
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                entry
                    .endpoint_env
                    .and_then(lookup_env)
                    .filter(|value| !value.trim().is_empty())
            });

        match entry.auth_mode {
            LoginAuthMode::None => {
                println!("Provider '{}' does not require login.", entry.name);
                return Ok(());
            }
            LoginAuthMode::EndpointAndKey => {
                let endpoint = resolved_endpoint.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Provider '{}' requires endpoint. Set --endpoint or {}.",
                        entry.name,
                        entry.endpoint_env.unwrap_or("PROVIDER_ENDPOINT")
                    )
                })?;
                let api_key = resolved_api_key.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Provider '{}' requires API key. Set --api-key or {}.",
                        entry.name,
                        entry.api_key_env
                    )
                })?;
                AuthLoginIntent {
                    provider: entry.name.to_string(),
                    auth_method: AuthMethodChoice::ApiKey,
                    endpoint: Some(endpoint),
                    api_key: Some(api_key),
                }
            }
            LoginAuthMode::ApiKey => {
                let api_key = resolved_api_key.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Provider '{}' requires API key. Set --api-key, {}, or openpista_API_KEY.",
                        entry.name,
                        entry.api_key_env
                    )
                })?;
                AuthLoginIntent {
                    provider: entry.name.to_string(),
                    auth_method: AuthMethodChoice::ApiKey,
                    endpoint: resolved_endpoint,
                    api_key: Some(api_key),
                }
            }
            LoginAuthMode::OAuth => {
                let method = if resolved_api_key.is_some() {
                    AuthMethodChoice::ApiKey
                } else {
                    AuthMethodChoice::OAuth
                };
                if method == AuthMethodChoice::OAuth
                    && !crate::config::oauth_available_for(
                        entry.name,
                        &config.agent.oauth_client_id,
                    )
                {
                    anyhow::bail!(
                        "No OAuth client ID configured for '{}'. Set openpista_OAUTH_CLIENT_ID or provide --api-key for API-key fallback.",
                        entry.name
                    );
                }
                AuthLoginIntent {
                    provider: entry.name.to_string(),
                    auth_method: method,
                    endpoint: resolved_endpoint,
                    api_key: resolved_api_key,
                }
            }
        }
    } else {
        let intent = auth_picker::run_cli_auth_picker(
            provider.as_deref(),
            config.agent.oauth_client_id.clone(),
        )?;
        let Some(intent) = intent else {
            println!("Login cancelled.");
            return Ok(());
        };
        intent
    };

    let message = persist_cli_auth_intent(&config, intent, port, timeout).await?;
    println!("\n{message}");
    Ok(())
}

#[cfg(not(test))]
async fn persist_cli_auth_intent(
    config: &Config,
    intent: AuthLoginIntent,
    port: u16,
    timeout: u64,
) -> anyhow::Result<String> {
    tui::event::build_and_store_credential(config, intent, port, timeout)
        .await
        .map_err(anyhow::Error::msg)
}

#[cfg(not(test))]
/// Removes stored credentials for `provider`.
fn cmd_auth_logout(provider: String) -> anyhow::Result<()> {
    let mut creds = auth::Credentials::load();
    if creds.remove(&provider) {
        creds.save()?;
        println!("Logged out from '{provider}'. Credentials removed.");
    } else {
        println!("No stored credentials found for '{provider}'.");
    }
    Ok(())
}

#[cfg(not(test))]
/// Prints the current authentication status for all stored providers.
fn cmd_auth_status() -> anyhow::Result<()> {
    let creds = auth::Credentials::load();
    if creds.providers.is_empty() {
        println!("No stored credentials. Run `openpista auth login` to authenticate.");
        return Ok(());
    }
    println!(
        "Stored credentials ({}):\n",
        auth::Credentials::path().display()
    );
    for (provider, cred) in &creds.providers {
        let status = match cred.expires_at {
            None => "valid (no expiry)".to_string(),
            Some(t) if t > chrono::Utc::now() => {
                format!("valid until {}", t.format("%Y-%m-%d %H:%M UTC"))
            }
            Some(t) => format!("EXPIRED at {}", t.format("%Y-%m-%d %H:%M UTC")),
        };
        println!("  {provider}: {status}");
    }
    Ok(())
}

/// Prints the branded farewell banner with session resume instructions.
fn print_goodbye_banner(session_id: &SessionId, model: &str) {
    let session_str = session_id.as_str();

    println!();
    println!("  \x1b[1;32m                            _     _      \x1b[0m");
    println!("  \x1b[1;32m  ___  _ __  ___ _ __  _ __(_)___| |_ __ _\x1b[0m");
    println!("  \x1b[1;32m / _ \\| '_ \\/ _ \\ '_ \\| '_ \\| / __| __/ _` |\x1b[0m");
    println!("  \x1b[1;32m| (_) | |_) |  __/ | | | |_) | \\__ \\ || (_| |\x1b[0m");
    println!("  \x1b[1;32m \\___/| .__/ \\___|_| |_| .__/|_|___/\\__\\__,_|\x1b[0m");
    println!("  \x1b[1;32m      |_|              |_|                   \x1b[0m");
    println!();
    println!(
        "  \x1b[1;37mSession\x1b[0m   \x1b[32m{}\x1b[0m",
        session_str
    );
    println!("  \x1b[1;37mModel\x1b[0m     \x1b[32m{}\x1b[0m", model);
    println!();
    println!(
        "  \x1b[1;37mContinue\x1b[0m  \x1b[1;32mopenpista -s {}\x1b[0m",
        session_str
    );
    println!();
}

/// Returns whether a response should be routed to Telegram.
fn should_send_telegram_response(channel_id: &ChannelId) -> bool {
    channel_id.as_str().starts_with("telegram:")
}

/// Returns whether a response should be routed to CLI.
fn should_send_cli_response(channel_id: &ChannelId) -> bool {
    channel_id.as_str().starts_with("cli:")
}

fn should_send_whatsapp_response(channel_id: &ChannelId) -> bool {
    channel_id.as_str().starts_with("whatsapp:")
}

fn should_send_web_response(channel_id: &ChannelId) -> bool {
    channel_id.as_str().starts_with("web:")
}

/// Builds an outbound response from runtime result.
fn build_agent_response(
    event: &ChannelEvent,
    result: Result<String, proto::Error>,
) -> AgentResponse {
    match result {
        Ok(text) => AgentResponse::new(event.channel_id.clone(), event.session_id.clone(), text),
        Err(e) => AgentResponse::error(
            event.channel_id.clone(),
            event.session_id.clone(),
            e.to_string(),
        ),
    }
}

/// Formats run mode header text.
fn format_run_header(exec: &str) -> String {
    format!("Running: {exec}")
}

fn prompt_whatsapp_model_warning(config: &Config) -> anyhow::Result<bool> {
    if !config.agent.model.is_empty() {
        return Ok(true);
    }

    let provider = config.agent.provider.name();
    let effective_model = config.agent.effective_model().to_string();

    if effective_model.is_empty() {
        println!("\u{26a0} No model configured for provider `{provider}`.");
        println!("  WhatsApp needs an LLM model to respond to messages.");
    } else {
        println!(
            "\u{26a0} No explicit model configured; using provider default {provider}/{effective_model}."
        );
    }
    println!();
    println!("  Run `openpista model select` to choose a model explicitly.");
    println!("  Or set it in ~/.openpista/config.toml:");
    println!("    [agent]");
    println!("    provider = \"anthropic\"");
    println!("    model = \"claude-sonnet-4-6\"");
    println!();
    print!("  Continue anyway? (y/N): ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        return Ok(false);
    }
    println!();
    Ok(true)
}

#[cfg(not(test))]
/// Non-TUI WhatsApp setup: check prerequisites, install bridge deps, spawn bridge,
/// display QR in terminal, and save config on successful connection.
async fn cmd_whatsapp(mut config: Config) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt;

    println!("WhatsApp Setup");
    println!("==============");
    println!();

    // 1. Check Node.js
    print!("Checking Node.js... ");
    let node_ok = tokio::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !node_ok {
        println!("NOT FOUND");
        anyhow::bail!(
            "Node.js is required for the WhatsApp bridge. Install it from https://nodejs.org/"
        );
    }
    println!("OK");

    // 2. Check / install bridge dependencies
    let bridge_installed = std::path::Path::new("whatsapp-bridge/node_modules").exists();
    if !bridge_installed {
        println!("Installing bridge dependencies (npm install)...");
        let status = tokio::process::Command::new("npm")
            .arg("install")
            .current_dir("whatsapp-bridge")
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("npm install failed with {status}");
        }
        println!("Dependencies installed.");
    } else {
        println!("Bridge dependencies: OK");
    }
    println!();

    // 3. Spawn bridge subprocess
    let bridge_path = config
        .channels
        .whatsapp
        .bridge_path
        .clone()
        .unwrap_or_else(|| "whatsapp-bridge/index.js".to_string());
    let session_dir = config.channels.whatsapp.session_dir.clone();

    println!("Starting WhatsApp bridge...");
    println!("Session dir: {session_dir}");
    println!("Bridge path: {bridge_path}");
    println!();

    let mut child = tokio::process::Command::new("node")
        .arg(&bridge_path)
        .arg(&session_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = child.stdout.take().expect("bridge stdout");
    let stderr = child.stderr.take().expect("bridge stderr");
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();
    // Drain stderr in background so the bridge can't block on a full pipe.
    let _stderr_drain = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut err_lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(_)) = err_lines.next_line().await {}
    });

    // 4. Read bridge events
    println!("Waiting for QR code... (scan with your phone)");
    println!();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if let Ok(event) = serde_json::from_str::<channels::whatsapp::BridgeEvent>(&line) {
                    match event {
                        channels::whatsapp::BridgeEvent::Qr { data } => {
                            if let Some(qr_text) = tui::event::render_qr_text(&data) {
                                println!("{qr_text}");
                            } else {
                                println!("QR data: {data}");
                            }
                            println!();
                            println!("Scan this QR code with WhatsApp on your phone.");
                            println!(
                                "(Open WhatsApp > Settings > Linked Devices > Link a Device)"
                            );
                            println!();
                        }
                        channels::whatsapp::BridgeEvent::Connected { phone, name } => {
                            let display_name = name.unwrap_or_default();
                            println!("Connected to WhatsApp!");
                            println!("  Phone: {phone}");
                            if !display_name.is_empty() {
                                println!("  Name:  {display_name}");
                            }
                            println!();

                            // 5. Save config
                            config.channels.whatsapp.enabled = true;
                            if let Err(e) = config.save() {
                                eprintln!("Warning: failed to save config: {e}");
                            } else {
                                println!("WhatsApp enabled in config.toml.");
                                println!(
                                    "Run `openpista start` to keep WhatsApp active in daemon mode."
                                );
                            }
                            break;
                        }
                        channels::whatsapp::BridgeEvent::Error { message } => {
                            eprintln!("Bridge error: {message}");
                        }
                        channels::whatsapp::BridgeEvent::Disconnected { reason } => {
                            let reason = reason.unwrap_or_else(|| "unknown".to_string());
                            if reason == "logged out" {
                                eprintln!("WhatsApp logged out. Session cleared.");
                                break;
                            }
                            // Transient disconnect — bridge will auto-reconnect
                            eprintln!("Bridge disconnected: {reason} (reconnecting...)");
                        }
                        _ => {}
                    }
                }
            }
            Ok(None) => {
                eprintln!("Bridge process exited unexpectedly.");
                break;
            }
            Err(e) => {
                eprintln!("Error reading bridge output: {e}");
                break;
            }
        }
    }

    // 6. Wait for credentials to be fully persisted before shutdown
    println!("Waiting for session to stabilize...");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // 7. Graceful shutdown — send shutdown command to bridge to preserve session
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let cmd = serde_json::json!({"type": "shutdown"});
        let line = format!("{}\n", cmd);
        let _ = stdin.write_all(line.as_bytes()).await;
        let _ = stdin.flush().await;
    }
    // Wait for bridge to exit gracefully (up to 5s), then force kill
    match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(_) => {}
        Err(_) => {
            let _ = child.kill().await;
        }
    }
    Ok(())
}

#[cfg(not(test))]
/// Shows WhatsApp connection status: config, session, credentials, phone number.
async fn cmd_whatsapp_status(config: Config) -> anyhow::Result<()> {
    println!("WhatsApp Status");
    println!("===============");
    println!();

    let enabled = config.channels.whatsapp.enabled;
    let session_dir = &config.channels.whatsapp.session_dir;
    let creds_path = format!("{session_dir}/auth/creds.json");
    let creds_exist = std::path::Path::new(&creds_path).exists();

    println!(
        "Config:      {}",
        if enabled { "enabled" } else { "disabled" }
    );
    println!("Session:     {session_dir}");

    if creds_exist {
        println!("Credentials: found (auth/creds.json exists)");
        if let Ok(data) = std::fs::read_to_string(&creds_path)
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&data)
        {
            if let Some(me_id) = json
                .get("me")
                .and_then(|me| me.get("id"))
                .and_then(|id| id.as_str())
            {
                let phone = me_id.split(':').next().unwrap_or(me_id);
                println!("Phone:       {phone}");
                println!("Link:        https://wa.me/{phone}");
            }
        }
    } else {
        println!("Credentials: not found");
    }

    println!();
    if creds_exist {
        println!("To start the bridge: openpista whatsapp start");
        println!("To re-pair (new QR): openpista whatsapp setup");
    } else {
        println!("Run `openpista whatsapp setup` to pair via QR code.");
    }
    Ok(())
}

#[cfg(not(test))]
/// Starts the WhatsApp bridge in foreground mode as an AI bot.
async fn cmd_whatsapp_start(config: Config) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::signal;

    println!("WhatsApp Bridge");
    println!("===============");
    println!();
    println!("\u{1F4A1} Tip: If you have trouble connecting, try disabling your VPN first.");
    println!("     VPN can block WhatsApp Web connections.");
    println!();

    if !prompt_whatsapp_model_warning(&config)? {
        return Ok(());
    }
    let effective_model = config.agent.effective_model().to_string();

    let node_ok = tokio::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !node_ok {
        anyhow::bail!(
            "Node.js is required for the WhatsApp bridge. Install it from https://nodejs.org/"
        );
    }

    if !std::path::Path::new("whatsapp-bridge/node_modules").exists() {
        anyhow::bail!("Bridge dependencies not installed. Run `openpista whatsapp setup` first.");
    }

    let session_dir = &config.channels.whatsapp.session_dir;
    let creds_path = format!("{session_dir}/auth/creds.json");
    if !std::path::Path::new(&creds_path).exists() {
        anyhow::bail!(
            "No WhatsApp session found. Run `openpista whatsapp setup` first to pair via QR code."
        );
    }

    let runtime = build_runtime(&config, Arc::new(AutoApproveHandler)).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));

    let bridge_path = config
        .channels
        .whatsapp
        .bridge_path
        .clone()
        .unwrap_or_else(|| "whatsapp-bridge/index.js".to_string());

    println!("Starting bridge...");
    println!("Session : {session_dir}");
    println!("Provider: {}", config.agent.provider.name());
    println!("Model   : {effective_model}");
    println!();

    let mut child = tokio::process::Command::new("node")
        .arg(&bridge_path)
        .arg(session_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = child.stdout.take().expect("bridge stdout");
    let stderr = child.stderr.take().expect("bridge stderr");
    let stdin = child.stdin.take().expect("bridge stdin");
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut err_lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(err_line)) = err_lines.next_line().await {
            tracing::debug!(target: "whatsapp_bridge", "{}", err_line);
        }
    });
    let stdin_shared = Arc::new(tokio::sync::Mutex::new(stdin));

    let shutdown_result = loop {
        tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Ok(event) = serde_json::from_str::<channels::whatsapp::BridgeEvent>(&line) {
                            match event {
                                channels::whatsapp::BridgeEvent::Qr { data } => {
                                    if let Some(qr_text) = tui::event::render_qr_text(&data) {
                                        println!("{qr_text}");
                                    }
                                    println!("Session expired. Scan QR code to re-pair.");
                                    println!();
                                }
                                channels::whatsapp::BridgeEvent::Connected { phone, name } => {
                                    let display = name.unwrap_or_default();
                                    println!("Connected: {phone} {display}");
                                    println!("Bot is running. Listening for messages... (Ctrl+C to stop)");
                                    println!();
                                }
                                channels::whatsapp::BridgeEvent::Message { from, text, .. } => {
                                    println!("[IN]  {from}: {text}");
                                    let runtime = runtime.clone();
                                    let skill_loader = skill_loader.clone();
                                    let stdin = stdin_shared.clone();
                                    let from_clone = from.clone();
                                    tokio::spawn(async move {
                                        let channel_id = ChannelId::new("whatsapp", &from_clone);
                                        let session_id =
                                            SessionId::from(format!("whatsapp:{from_clone}"));
                                        let skills_ctx = skill_loader.load_context().await;
                                        let result = runtime
                                            .process(
                                                &channel_id,
                                                &session_id,
                                                &text,
                                                Some(&skills_ctx),
                                            )
                                            .await;
                                        let reply_text = match result {
                                            Ok(text) => text,
                                            Err(e) => format!("Error: {e}"),
                                        };
                                        println!("[OUT] {from_clone}: {reply_text}");
                                        let jid = if from_clone.contains('@') {
                                            from_clone.clone()
                                        } else {
                                            format!("{from_clone}@s.whatsapp.net")
                                        };
                                        let cmd = serde_json::json!({
                                            "type": "send",
                                            "to": jid,
                                            "text": reply_text
                                        });
                                        let line = format!("{}\n", cmd);
                                        let mut guard = stdin.lock().await;
                                        let _ = guard.write_all(line.as_bytes()).await;
                                        let _ = guard.flush().await;
                                    });
                                }
                                channels::whatsapp::BridgeEvent::Disconnected { reason } => {
                                    let reason = reason.unwrap_or_else(|| "unknown".to_string());
                                    if reason == "logged out" {
                                        eprintln!("Session logged out. Run `openpista whatsapp setup` to re-pair.");
                                        break Ok(());
                                    }
                                    eprintln!("Disconnected: {reason} (bridge reconnecting...)");
                                }
                                channels::whatsapp::BridgeEvent::Error { message } => {
                                    eprintln!("Bridge error: {message}");
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        eprintln!("Bridge process exited.");
                        break Ok(());
                    }
                    Err(e) => {
                        break Err(anyhow::anyhow!("Error reading bridge output: {e}"));
                    }
                }
            }
            _ = signal::ctrl_c() => {
                println!();
                println!("Shutting down gracefully...");
                break Ok(());
            }
        }
    };

    {
        let cmd = serde_json::json!({"type": "shutdown"});
        let line = format!("{}\n", cmd);
        let mut guard = stdin_shared.lock().await;
        let _ = guard.write_all(line.as_bytes()).await;
        let _ = guard.flush().await;
    }
    match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(_) => {}
        Err(_) => {
            let _ = child.kill().await;
        }
    }
    shutdown_result
}

#[cfg(not(test))]
/// Send a single message to a WhatsApp number and exit.
async fn cmd_whatsapp_send(config: Config, number: String, message: String) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    if message.trim().is_empty() {
        anyhow::bail!("Message cannot be empty.");
    }

    let node_ok = tokio::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !node_ok {
        anyhow::bail!("Node.js is required. Install from https://nodejs.org/");
    }

    let session_dir = &config.channels.whatsapp.session_dir;
    let creds_path = format!("{session_dir}/auth/creds.json");
    if !std::path::Path::new(&creds_path).exists() {
        anyhow::bail!("No WhatsApp session. Run `openpista whatsapp setup` first.");
    }

    let bridge_path = config
        .channels
        .whatsapp
        .bridge_path
        .clone()
        .unwrap_or_else(|| "whatsapp-bridge/index.js".to_string());

    let mut child = tokio::process::Command::new("node")
        .arg(&bridge_path)
        .arg(session_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = child.stdout.take().expect("bridge stdout");
    let stderr = child.stderr.take().expect("bridge stderr");
    let mut stdin = child.stdin.take().expect("bridge stdin");
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();
    let _stderr_drain = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut err_lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(_)) = err_lines.next_line().await {}
    });

    println!("Connecting to WhatsApp...");

    let result = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(30), lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                if let Ok(event) = serde_json::from_str::<channels::whatsapp::BridgeEvent>(&line) {
                    match event {
                        channels::whatsapp::BridgeEvent::Connected { .. } => {
                            let jid = if number.contains('@') {
                                number.clone()
                            } else {
                                format!("{number}@s.whatsapp.net")
                            };
                            let cmd = serde_json::json!({
                                "type": "send",
                                "to": jid,
                                "text": message
                            });
                            let json_line = format!("{}\n", cmd);
                            stdin.write_all(json_line.as_bytes()).await?;
                            stdin.flush().await?;
                            println!("Message sent to {number}: {message}");
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            break Ok(());
                        }
                        channels::whatsapp::BridgeEvent::Disconnected { reason } => {
                            let r = reason.unwrap_or_else(|| "unknown".to_string());
                            if r == "logged out" {
                                break Err(anyhow::anyhow!(
                                    "Session logged out. Run `openpista whatsapp setup` to re-pair."
                                ));
                            }
                        }
                        channels::whatsapp::BridgeEvent::Error { message } => {
                            break Err(anyhow::anyhow!("Bridge error: {message}"));
                        }
                        _ => {}
                    }
                }
            }
            Ok(Ok(None)) => break Err(anyhow::anyhow!("Bridge exited before connecting.")),
            Ok(Err(e)) => break Err(anyhow::anyhow!("Bridge read error: {e}")),
            Err(_) => break Err(anyhow::anyhow!("Timeout waiting for WhatsApp connection.")),
        }
    };

    let _ = child.kill().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn should_send_telegram_response_checks_prefix() {
        assert!(should_send_telegram_response(&ChannelId::from(
            "telegram:123"
        )));
        assert!(!should_send_telegram_response(&ChannelId::from(
            "cli:local"
        )));
    }

    #[test]
    fn should_send_cli_response_checks_prefix() {
        assert!(should_send_cli_response(&ChannelId::from("cli:local")));
        assert!(!should_send_cli_response(&ChannelId::from("telegram:123")));
    }

    fn should_send_whatsapp_response_checks_prefix() {
        assert!(should_send_whatsapp_response(&ChannelId::from(
            "whatsapp:123"
        )));
        assert!(!should_send_whatsapp_response(&ChannelId::from(
            "cli:local"
        )));
    }

    #[test]
    fn should_send_web_response_checks_prefix() {
        assert!(should_send_web_response(&ChannelId::from("web:abc")));
        assert!(!should_send_web_response(&ChannelId::from("telegram:123")));
        assert!(!should_send_web_response(&ChannelId::from("cli:local")));
    }

    #[test]
    fn cli_parses_web_setup_command() {
        let cli = Cli::try_parse_from(["openpista", "web", "setup", "--port", "3211", "--enable"])
            .expect("parse web setup");

        match cli.command {
            Some(Commands::Web { command }) => match command {
                WebCommands::Setup { port, enable, .. } => {
                    assert_eq!(port, Some(3211));
                    assert!(enable);
                }
                _ => panic!("expected web setup command"),
            },
            _ => panic!("expected web command"),
        }
    }

    #[test]
    fn cli_parses_web_setup_regenerate_token_and_yes_flags() {
        let cli = Cli::try_parse_from(["openpista", "web", "setup", "--regenerate-token", "--yes"])
            .expect("parse web setup with token flags");

        match cli.command {
            Some(Commands::Web { command }) => match command {
                WebCommands::Setup {
                    regenerate_token,
                    yes,
                    ..
                } => {
                    assert!(regenerate_token);
                    assert!(yes);
                }
                _ => panic!("expected web setup command"),
            },
            _ => panic!("expected web command"),
        }
    }

    #[test]
    fn cli_parses_web_setup_shared_session_id_flag() {
        let cli = Cli::try_parse_from([
            "openpista",
            "web",
            "setup",
            "--shared-session-id",
            "team-room",
        ])
        .expect("parse web setup shared session flag");

        match cli.command {
            Some(Commands::Web { command }) => match command {
                WebCommands::Setup {
                    shared_session_id, ..
                } => {
                    assert_eq!(shared_session_id.as_deref(), Some("team-room"));
                }
                _ => panic!("expected web setup command"),
            },
            _ => panic!("expected web command"),
        }
    }

    #[test]
    fn cli_rejects_web_setup_token_and_regenerate_token_together() {
        let result = Cli::try_parse_from([
            "openpista",
            "web",
            "setup",
            "--token",
            "abc",
            "--regenerate-token",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_parses_web_start_command() {
        let cli =
            Cli::try_parse_from(["openpista", "web", "start"]).expect("parse web start command");
        match cli.command {
            Some(Commands::Web { command }) => {
                assert!(matches!(command, WebCommands::Start));
            }
            _ => panic!("expected web start command"),
        }
    }

    #[test]
    fn cli_rejects_web_stop_and_restart_commands() {
        let stop = Cli::try_parse_from(["openpista", "web", "stop"]);
        assert!(stop.is_err());

        let restart = Cli::try_parse_from(["openpista", "web", "restart"]);
        assert!(restart.is_err());
    }

    #[test]
    fn cli_parses_web_status_command() {
        let cli =
            Cli::try_parse_from(["openpista", "web", "status"]).expect("parse web status command");
        match cli.command {
            Some(Commands::Web { command }) => {
                assert!(matches!(command, WebCommands::Status));
            }
            _ => panic!("expected web status command"),
        }
    }

    #[test]
    fn cli_parses_model_select_command() {
        let cli =
            Cli::try_parse_from(["openpista", "model", "select"]).expect("parse model select");
        match cli.command {
            Some(Commands::Model { model_name, .. }) => {
                assert_eq!(model_name.as_deref(), Some("select"));
            }
            _ => panic!("expected model command"),
        }
    }

    #[test]
    fn build_agent_response_maps_success_and_error() {
        let event = ChannelEvent::new(ChannelId::from("cli:local"), SessionId::from("s1"), "msg");
        let ok = build_agent_response(&event, Ok("done".to_string()));
        assert_eq!(ok.content, "done");
        assert!(!ok.is_error);

        let err = build_agent_response(&event, Err(proto::Error::Llm(proto::LlmError::RateLimit)));
        assert!(err.is_error);
        assert!(err.content.contains("Rate limit exceeded"));
    }

    #[test]
    fn format_run_header_embeds_exec_text() {
        assert_eq!(format_run_header("ls -la"), "Running: ls -la");
    }

    #[test]
    fn resolve_tui_session_id_uses_explicit_session_first() {
        let mut config = Config::default();
        config.channels.web.shared_session_id = "shared-main".to_string();

        let session = resolve_tui_session_id(&config, Some("manual-session".to_string()));
        assert_eq!(session.as_str(), "manual-session");
    }

    #[test]
    fn resolve_tui_session_id_uses_configured_shared_session_when_not_explicit() {
        let mut config = Config::default();
        config.channels.web.shared_session_id = "shared-main".to_string();

        let session = resolve_tui_session_id(&config, None);
        assert_eq!(session.as_str(), "shared-main");
    }

    #[test]
    fn resolve_tui_session_id_generates_new_when_shared_session_is_empty() {
        let mut config = Config::default();
        config.channels.web.shared_session_id.clear();

        let session = resolve_tui_session_id(&config, None);
        assert!(!session.as_str().is_empty());
        assert_ne!(session.as_str(), "shared-main");
    }

    #[test]
    fn print_goodbye_banner_does_not_panic() {
        let session = SessionId::from("abcdef12-3456-7890-abcd-ef1234567890");
        print_goodbye_banner(&session, "gpt-4o");
    }

    #[test]
    fn print_goodbye_banner_short_session_id() {
        let session = SessionId::from("short");
        print_goodbye_banner(&session, "claude-sonnet-4-20250514");
    }

    // ─── Telegram CLI tests ─────────────────────────────────────────

    #[test]
    fn is_valid_telegram_token_accepts_valid() {
        assert!(is_valid_telegram_token("123456:ABCdef"));
        assert!(is_valid_telegram_token("9999999:xYz_123-ABC"));
    }

    #[test]
    fn is_valid_telegram_token_rejects_invalid() {
        assert!(!is_valid_telegram_token("notokens"));
        assert!(!is_valid_telegram_token("abc:def")); // non-numeric prefix
        assert!(!is_valid_telegram_token("123456:")); // empty suffix
        assert!(!is_valid_telegram_token(":ABCdef")); // empty prefix
        assert!(!is_valid_telegram_token(""));
    }

    #[test]
    fn telegram_status_logic_not_configured() {
        let config = Config::default();
        assert!(!config.channels.telegram.enabled);
        assert!(config.channels.telegram.token.is_empty());
    }

    #[test]
    fn telegram_status_logic_enabled_with_token() {
        let mut config = Config::default();
        config.channels.telegram.enabled = true;
        config.channels.telegram.token = "123456:ABC".to_string();
        assert!(config.channels.telegram.enabled);
        assert!(!config.channels.telegram.token.is_empty());
    }

    #[test]
    fn telegram_status_logic_token_but_disabled() {
        let mut config = Config::default();
        config.channels.telegram.enabled = false;
        config.channels.telegram.token = "123456:ABC".to_string();
        assert!(!config.channels.telegram.enabled);
        assert!(!config.channels.telegram.token.is_empty());
    }
}
