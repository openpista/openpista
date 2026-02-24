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
use std::sync::Arc;

#[cfg(not(test))]
use crate::auth::is_openai_oauth_credential_for_key;
#[cfg(not(test))]
use crate::auth_picker::{AuthLoginIntent, AuthMethodChoice};
#[cfg(not(test))]
use agent::{
    AgentRuntime, AnthropicProvider, OpenAiProvider, ResponsesApiProvider, SqliteMemory,
    ToolRegistry,
};
#[cfg(not(test))]
use channels::{ChannelAdapter, CliAdapter, TelegramAdapter, WebAdapter, WhatsAppAdapter};
#[cfg(not(test))]
use config::Config;
#[cfg(not(test))]
use skills::SkillLoader;
#[cfg(not(test))]
use tools::{
    BashTool, BrowserClickTool, BrowserScreenshotTool, BrowserTool, BrowserTypeTool, ContainerTool,
    ScreenTool,
};
#[cfg(not(test))]
use tracing::{error, info, warn};
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
    /// Set up WhatsApp channel (check prerequisites, install bridge, pair via QR)
    Whatsapp,
    /// Manage Telegram channel (status, setup guide, enable)
    Telegram {
        #[command(subcommand)]
        command: TelegramCommands,
    },
}

/// `auth` sub-subcommands.
/// `telegram` sub-subcommands.
#[derive(Subcommand)]
enum TelegramCommands {
    /// Show current Telegram channel configuration and readiness
    Status,
    /// Print step-by-step guide to create a bot via @BotFather
    Setup,
    /// Save bot token to config and enable the Telegram channel
    Start {
        /// Bot token from @BotFather (e.g. 123456:ABC...)
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
            Commands::Whatsapp => "whatsapp",
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
            None | Some("list") => cmd_models(config).await,
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
        Commands::Whatsapp => cmd_whatsapp(config).await,
        Commands::Telegram { command } => match command {
            TelegramCommands::Status => cmd_telegram_status(&config),
            TelegramCommands::Setup => cmd_telegram_setup(),
            TelegramCommands::Start { token } => cmd_telegram_start(config, token).await,
        },
    }
}

#[cfg(not(test))]
/// Starts the full-screen TUI for interactive agent sessions.
async fn cmd_tui(config: Config, session: Option<String>) -> anyhow::Result<()> {
    let runtime = build_runtime(&config).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));
    let channel_id = ChannelId::new("cli", "tui");
    let session_id = match session {
        Some(id) => SessionId::from(id),
        None => SessionId::new(),
    };
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
    )
    .await?;

    print_goodbye_banner(&session_id, config.agent.effective_model());
    Ok(())
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
async fn build_runtime(config: &Config) -> anyhow::Result<Arc<AgentRuntime>> {
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

    let runtime = build_runtime(&config).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));

    // In-process event bus
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<ChannelEvent>(128);
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

    // Web adapter
    let mut web_resp_adapter: Option<WebAdapter> = None;
    if config.channels.web.enabled {
        let web_config = channels::web::WebAdapterConfig {
            port: config.channels.web.port,
            token: config.channels.web.token.clone(),
            cors_origins: config.channels.web.cors_origins.clone(),
            static_dir: config.channels.web.static_dir.clone(),
        };
        let tx = event_tx.clone();
        let adapter = WebAdapter::new(web_config, resp_tx.clone());
        web_resp_adapter = Some(adapter.clone_for_responses());

        tokio::spawn(async move {
            if let Err(e) = adapter.run(tx).await {
                error!("Web adapter error: {e}");
            }
        });
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
    tokio::select! {
        _ = async {
            while let Some(event) = event_rx.recv().await {
                let runtime = runtime.clone();
                let skill_loader = skill_loader.clone();
                let resp_tx = resp_tx.clone();

                tokio::spawn(async move {
                    let skills_ctx = skill_loader.load_context().await;
                    let result = runtime
                        .process(
                            &event.channel_id,
                            &event.session_id,
                            &event.user_message,
                            Some(&skills_ctx),
                        )
                        .await;

                    let resp = build_agent_response(&event, result);

                    let _ = resp_tx.send(resp).await;
                });
            }
        } => {}
        _ = daemon::wait_for_shutdown() => {
            info!("Shutdown signal received");
        }
    }

    pid_file.remove().await;
    info!("openpista stopped");
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

    println!("  Enabled : {}", if enabled { "Yes" } else { "No" });
    println!(
        "  Token   : {}",
        if token_set { "(set)" } else { "(not set)" }
    );
    println!();

    match (enabled, token_set) {
        (true, true) => println!("  Status  : Ready — run `openpista start` to activate."),
        (false, true) => {
            println!("  Status  : Token set but channel is disabled.");
            println!("           Enable it in config.toml or run `openpista telegram start`.");
        }
        (_, false) => {
            println!("  Status  : Not configured.");
            println!("           Run `openpista telegram setup` for setup instructions.");
        }
    }
    Ok(())
}

/// `openpista telegram setup` — prints a step-by-step bot creation guide.
#[cfg(not(test))]
fn cmd_telegram_setup() -> anyhow::Result<()> {
    println!("Telegram Setup Guide");
    println!("====================");
    println!();
    println!("1. Open Telegram and search for @BotFather");
    println!("2. Send /newbot and follow the prompts");
    println!("3. Copy the bot token (format: 123456:ABC...)");
    println!();
    println!("Then run:");
    println!("  openpista telegram start --token YOUR_TOKEN");
    println!();
    println!("Or add it manually to config.toml:");
    println!("  [channels.telegram]");
    println!("  enabled = true");
    println!("  token   = \"123456:ABC...\"");
    println!();
    println!("Or use an environment variable (daemon mode):");
    println!("  TELEGRAM_BOT_TOKEN=123456:ABC... openpista start");
    Ok(())
}

/// `openpista telegram start [--token TOKEN]` — saves token and enables Telegram.
#[cfg(not(test))]
async fn cmd_telegram_start(mut config: Config, token: Option<String>) -> anyhow::Result<()> {
    // Resolve token: flag > env var > already in config
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
            eprintln!("Error: no bot token provided.");
            eprintln!();
            eprintln!("Supply one with --token or TELEGRAM_BOT_TOKEN, or run:");
            eprintln!("  openpista telegram setup");
            anyhow::bail!("missing Telegram bot token");
        }
    };

    if !is_valid_telegram_token(&token) {
        anyhow::bail!(
            "Invalid token format '{}'. Expected NUMBERS:STRING (e.g. 123456:ABC...)",
            token
        );
    }

    config.channels.telegram.token = token.clone();
    config.channels.telegram.enabled = true;

    config
        .save()
        .map_err(|e| anyhow::anyhow!("Failed to save config: {e}"))?;

    println!("Telegram channel enabled.");
    println!("Token saved to config.toml.");
    println!();
    println!("Run `openpista start` to activate all channels.");
    Ok(())
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
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();

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
                            println!("(Open WhatsApp > Settings > Linked Devices > Link a Device)");
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

    // 6. Cleanup — kill_on_drop handles it, but be explicit
    let _ = child.kill().await;
    Ok(())
}

#[cfg(not(test))]
/// Executes one command against the agent and exits.
async fn cmd_run(config: Config, exec: String) -> anyhow::Result<()> {
    let runtime = build_runtime(&config).await?;
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

    let runtime = build_runtime(&config).await?;
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
        let runtime = match build_runtime(&test_config).await {
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

    #[test]
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
        assert!(should_send_web_response(&ChannelId::from("web:conn1")));
        assert!(!should_send_web_response(&ChannelId::from("telegram:123")));
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
