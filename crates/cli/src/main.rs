//! CLI entrypoint and subcommand orchestration.

#[cfg(not(test))]
const GITHUB_REPO_URL: &str = "https://github.com/openpista/openpista";

mod auth;
mod auth_picker;
mod cmd_auth;
mod cmd_model;
mod cmd_telegram;
mod cmd_web;
mod cmd_whatsapp;
mod config;
mod daemon;
mod model_catalog;
mod presets;
mod run_cmd;
mod startup;
#[cfg(test)]
mod test_support;
mod tui;

use clap::{Parser, Subcommand};
use proto::{AgentResponse, ChannelEvent, ChannelId, SessionId};

#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use agent::AutoApproveHandler;
#[cfg(not(test))]
use channels::{ChannelAdapter, CliAdapter, TelegramAdapter, WebAdapter, WhatsAppAdapter};
#[cfg(not(test))]
use config::Config;
#[cfg(not(test))]
use skills::SkillLoader;
#[cfg(not(test))]
use tracing::{error, info, warn};
#[cfg(not(test))]
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

// Re-imports from extracted modules
use startup::resolve_tui_session_id;

#[cfg(not(test))]
use cmd_auth::{cmd_auth_login, cmd_auth_logout, cmd_auth_status};
#[cfg(not(test))]
use cmd_model::{cmd_model_select, cmd_model_test, cmd_model_test_all, cmd_models};
#[cfg(not(test))]
use cmd_telegram::{cmd_telegram_setup, cmd_telegram_start, cmd_telegram_status};
#[cfg(not(test))]
use cmd_web::{
    WebSetupOptions, build_web_adapter, cmd_web_setup, cmd_web_start, cmd_web_status,
    is_interactive_terminal, prompt_yes_no, run_web_event_loop, spawn_web_session_sync_task,
};
#[cfg(not(test))]
use cmd_whatsapp::{cmd_whatsapp, cmd_whatsapp_send, cmd_whatsapp_start, cmd_whatsapp_status};
#[cfg(not(test))]
use run_cmd::cmd_run;
#[cfg(not(test))]
use startup::build_runtime;

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
    let persisted = config::TuiState::try_load();
    let is_first_run = persisted.is_none();
    let tui_state = persisted.unwrap_or_default();
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

    if is_first_run && is_interactive_terminal() {
        let answer = prompt_yes_no(
            "⭐  Enjoying openpista? Give us a star on GitHub! Open browser?",
            true,
        )?;
        if answer {
            auth::open_browser(GITHUB_REPO_URL);
        }
    }

    Ok(())
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
pub(crate) fn should_send_telegram_response(channel_id: &ChannelId) -> bool {
    channel_id.kind() == Some(proto::ChannelKind::Telegram)
}

/// Returns whether a response should be routed to CLI.
fn should_send_cli_response(channel_id: &ChannelId) -> bool {
    channel_id.kind() == Some(proto::ChannelKind::Cli)
}

fn should_send_whatsapp_response(channel_id: &ChannelId) -> bool {
    channel_id.kind() == Some(proto::ChannelKind::WhatsApp)
}

pub(crate) fn should_send_web_response(channel_id: &ChannelId) -> bool {
    channel_id.kind() == Some(proto::ChannelKind::Web)
}

/// Builds an outbound response from runtime result.
pub(crate) fn build_agent_response(
    event: &ChannelEvent,
    result: Result<(String, agent::TokenUsage), proto::Error>,
) -> AgentResponse {
    match result {
        Ok((text, _usage)) => {
            AgentResponse::new(event.channel_id.clone(), event.session_id.clone(), text)
        }
        Err(e) => AgentResponse::error(
            event.channel_id.clone(),
            event.session_id.clone(),
            e.to_string(),
        ),
    }
}

/// Formats run mode header text.
pub(crate) fn format_run_header(exec: &str) -> String {
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
        let ok = build_agent_response(
            &event,
            Ok(("done".to_string(), agent::TokenUsage::default())),
        );
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
        assert!(cmd_telegram::is_valid_telegram_token("123456:ABCdef"));
        assert!(cmd_telegram::is_valid_telegram_token("9999999:xYz_123-ABC"));
    }

    #[test]
    fn is_valid_telegram_token_rejects_invalid() {
        assert!(!cmd_telegram::is_valid_telegram_token("notokens"));
        assert!(!cmd_telegram::is_valid_telegram_token("abc:def")); // non-numeric prefix
        assert!(!cmd_telegram::is_valid_telegram_token("123456:")); // empty suffix
        assert!(!cmd_telegram::is_valid_telegram_token(":ABCdef")); // empty prefix
        assert!(!cmd_telegram::is_valid_telegram_token(""));
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
