//! CLI entrypoint and subcommand orchestration.

mod auth;
mod config;
mod daemon;
mod tui;

use clap::{Parser, Subcommand};
use proto::{AgentResponse, ChannelEvent, ChannelId, SessionId, WORKER_REPORT_KIND, WorkerReport};

#[cfg(not(test))]
use std::net::SocketAddr;
#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use agent::{AgentRuntime, OpenAiProvider, SqliteMemory, ToolRegistry};
#[cfg(not(test))]
use channels::{ChannelAdapter, CliAdapter, MobileAdapter, TelegramAdapter};
#[cfg(not(test))]
use config::Config;
#[cfg(not(test))]
use gateway::QuicServer;
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
use tracing_subscriber::{EnvFilter, fmt};

/// Parsed REPL line classification.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplInput {
    /// Empty or whitespace-only input line.
    Empty,
    /// Exit command (`/quit` or `/exit`).
    Exit,
    /// Normal user message content.
    Message(String),
}

/// Top-level command-line arguments for the openpista application.
#[derive(Parser)]
#[command(name = "openpista")]
#[command(about = "QUIC-based OS Gateway AI Agent", version = "0.1.0")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// CLI subcommands available in the application.
#[derive(Subcommand)]
enum Commands {
    /// Start the full-screen TUI (default when no subcommand is given)
    Tui,

    /// Start the daemon (gateway + all enabled channels)
    Start,

    /// Run a single command and exit
    Run {
        /// Command or message to send to the agent
        #[arg(short = 'e', long)]
        exec: String,
    },

    /// Start interactive REPL session
    Repl,

    /// Manage provider credentials via OAuth 2.0 PKCE browser login
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
}

/// `auth` sub-subcommands.
#[derive(Subcommand)]
enum AuthCommands {
    /// Authenticate with a provider via browser-based OAuth PKCE flow
    Login {
        /// Provider to authenticate with (openai, openrouter)
        #[arg(short, long, default_value = "openai")]
        provider: String,

        /// Local port for the OAuth callback server
        #[arg(long, default_value_t = 9009)]
        port: u16,

        /// Seconds to wait for the browser authorization before timing out
        #[arg(long, default_value_t = 120)]
        timeout: u64,
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
    let command = cli.command.unwrap_or(Commands::Tui);
    let is_tui = matches!(command, Commands::Tui);

    // Initialize tracing — suppress in TUI mode to avoid corrupting the display
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    if is_tui {
        fmt()
            .with_env_filter(filter)
            .with_writer(std::io::sink)
            .with_target(false)
            .init();
    } else {
        fmt().with_env_filter(filter).with_target(false).init();
    }

    // Load config
    let config = Config::load(cli.config.as_deref()).unwrap_or_else(|e| {
        warn!("Failed to load config ({e}), using defaults");
        Config::default()
    });

    match command {
        Commands::Tui => cmd_tui(config).await,
        Commands::Start => cmd_start(config).await,
        Commands::Run { exec } => cmd_run(config, exec).await,
        Commands::Repl => cmd_repl(config).await,
        Commands::Auth { command } => match command {
            AuthCommands::Login {
                provider,
                port,
                timeout,
            } => cmd_auth_login(config, provider, port, timeout).await,
            AuthCommands::Logout { provider } => cmd_auth_logout(provider),
            AuthCommands::Status => cmd_auth_status(),
        },
    }
}

#[cfg(not(test))]
/// Starts the full-screen TUI for interactive agent sessions.
async fn cmd_tui(config: Config) -> anyhow::Result<()> {
    let runtime = build_runtime(&config, None).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));
    let channel_id = ChannelId::new("cli", "tui");
    let session_id = SessionId::new();
    let model_name = config.agent.effective_model().to_string();

    tui::run_tui(runtime, skill_loader, channel_id, session_id, model_name).await
}

#[cfg(not(test))]
/// Creates a runtime with configured tools, memory, and LLM provider.
async fn build_runtime(
    config: &Config,
    worker_report_quic_addr: Option<String>,
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
    let api_key = config.resolve_api_key();
    if api_key.is_empty() {
        warn!("No API key configured. Set OPENPISTACRAB_API_KEY or OPENAI_API_KEY.");
    }

    let model = config.agent.effective_model().to_string();
    let llm: Arc<dyn agent::LlmProvider> = if let Some(base_url) = config.agent.effective_base_url()
    {
        Arc::new(OpenAiProvider::with_base_url(&api_key, base_url, &model))
    } else {
        Arc::new(OpenAiProvider::new(&api_key, &model))
    };

    Ok(Arc::new(
        AgentRuntime::new(llm, registry, memory, &model, config.agent.max_tool_rounds)
            .with_worker_report_quic_addr(worker_report_quic_addr),
    ))
}

#[cfg(not(test))]
/// Starts daemon mode with enabled channel adapters.
async fn cmd_start(config: Config) -> anyhow::Result<()> {
    info!("Starting openpistacrab daemon");

    let report_addr = format!("127.0.0.1:{}", config.gateway.port);
    let runtime = build_runtime(&config, Some(report_addr)).await?;
    let skill_loader = Arc::new(SkillLoader::new(&config.skills.workspace));

    // In-process event gateway
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<ChannelEvent>(128);
    let (resp_tx, mut resp_rx) = tokio::sync::mpsc::channel::<AgentResponse>(128);

    // Start QUIC server
    let addr: SocketAddr = format!("0.0.0.0:{}", config.gateway.port).parse()?;
    {
        let event_tx_quic = event_tx.clone();
        let runtime_quic = runtime.clone();

        let handler: gateway::AgentHandler = Arc::new(move |event: ChannelEvent| {
            let event_tx = event_tx_quic.clone();
            let runtime = runtime_quic.clone();
            Box::pin(async move {
                if let Some(report) = parse_worker_report(&event) {
                    let result = runtime
                        .record_worker_report(&event.channel_id, &event.session_id, &report)
                        .await;
                    return Some(match result {
                        Ok(()) => "worker-report-recorded".to_string(),
                        Err(e) => format!("worker-report-error:{e}"),
                    });
                }
                let _ = event_tx.send(event).await;
                Some("queued".to_string())
            })
        });

        match QuicServer::new_self_signed(addr, handler) {
            Ok(server) => {
                tokio::spawn(async move { server.run().await });
                info!("QUIC gateway listening on {addr}");
            }
            Err(e) => {
                warn!("Failed to start QUIC server: {e}. Continuing without QUIC.");
            }
        }
    }

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

    // Mobile QUIC adapter (if enabled)
    let mut mobile_resp_adapter: Option<MobileAdapter> = None;
    if config.channels.mobile.enabled {
        let token = config.channels.mobile.api_token.clone();
        if token.is_empty() {
            warn!("Mobile adapter enabled but no api_token configured");
        } else {
            let mobile_addr: SocketAddr =
                format!("0.0.0.0:{}", config.channels.mobile.port).parse()?;
            let adapter = MobileAdapter::new(mobile_addr, token);
            mobile_resp_adapter = Some(adapter.response_handle());
            let tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = adapter.run(tx).await {
                    error!("Mobile adapter error: {e}");
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

    // Response forwarder (always consume `resp_rx` to avoid dropped/backed-up responses)
    tokio::spawn(async move {
        while let Some(resp) = resp_rx.recv().await {
            let channel_id = resp.channel_id.clone();

            if should_send_mobile_response(&channel_id) {
                if let Some(adapter) = &mobile_resp_adapter {
                    if let Err(e) = adapter.send_response(resp).await {
                        error!("Failed to send mobile response: {e}");
                    }
                } else {
                    warn!("Mobile response dropped because mobile channel is disabled");
                }
                continue;
            }

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
    info!("openpistacrab stopped");
    Ok(())
}

#[cfg(not(test))]
/// Executes one command against the agent and exits.
async fn cmd_run(config: Config, exec: String) -> anyhow::Result<()> {
    let runtime = build_runtime(&config, None).await?;
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
/// Runs interactive REPL mode.
async fn cmd_repl(config: Config) -> anyhow::Result<()> {
    let runtime = build_runtime(&config, None).await?;
    let skill_loader = SkillLoader::new(&config.skills.workspace);

    let channel_id = ChannelId::new("cli", "repl");
    let session_id = SessionId::new();

    println!("openpistacrab REPL (session: {})", session_id);
    println!("Type /quit to exit\n");

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    loop {
        stdout.write_all(b"> ").await?;
        stdout.flush().await?;

        match reader.next_line().await? {
            None => break,
            Some(line) => {
                let line = match parse_repl_input(&line) {
                    ReplInput::Empty => continue,
                    ReplInput::Exit => break,
                    ReplInput::Message(text) => text,
                };

                let skills_ctx = skill_loader.load_context().await;

                match runtime
                    .process(&channel_id, &session_id, &line, Some(&skills_ctx))
                    .await
                {
                    Ok(text) => {
                        println!("\n{text}\n");
                    }
                    Err(e) => {
                        eprintln!("\nError: {e}\n");
                    }
                }
            }
        }
    }

    println!("Goodbye!");
    Ok(())
}

#[cfg(not(test))]
/// Runs the OAuth PKCE login flow for `provider` and persists the token.
async fn cmd_auth_login(
    config: Config,
    provider: String,
    port: u16,
    timeout: u64,
) -> anyhow::Result<()> {
    let preset: crate::config::ProviderPreset = provider
        .parse()
        .map_err(|_| anyhow::anyhow!("unknown provider '{provider}'"))?;

    let endpoints = preset.oauth_endpoints().ok_or_else(|| {
        anyhow::anyhow!(
            "provider '{provider}' does not support OAuth PKCE login.\n\
             Supported providers: openai, openrouter\n\
             For API-key-only providers (together, ollama), set api_key in config.toml."
        )
    })?;

    let client_id = if !config.agent.oauth_client_id.is_empty() {
        config.agent.oauth_client_id.clone()
    } else {
        anyhow::bail!(
            "No OAuth client ID configured for '{provider}'.\n\
             Register an OAuth app with the provider and set one of:\n\
             • agent.oauth_client_id in config.toml\n\
             • OPENPISTACRAB_OAUTH_CLIENT_ID environment variable"
        )
    };

    let cred = auth::login(&provider, &endpoints, &client_id, port, timeout).await?;

    let mut creds = auth::Credentials::load();
    creds.set(provider.clone(), cred);
    creds.save()?;

    println!(
        "\nAuthenticated as '{provider}'. Token stored in {}",
        auth::Credentials::path().display()
    );
    Ok(())
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
        println!("No stored credentials. Run `openpistacrab auth login` to authenticate.");
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

/// Parses one REPL input line into semantic input kind.
fn parse_repl_input(line: &str) -> ReplInput {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        ReplInput::Empty
    } else if trimmed == "/quit" || trimmed == "/exit" {
        ReplInput::Exit
    } else {
        ReplInput::Message(trimmed.to_string())
    }
}

/// Returns whether a response should be routed to the mobile QUIC adapter.
fn should_send_mobile_response(channel_id: &ChannelId) -> bool {
    channel_id.as_str().starts_with("mobile:")
}

/// Returns whether a response should be routed to Telegram.
fn should_send_telegram_response(channel_id: &ChannelId) -> bool {
    channel_id.as_str().starts_with("telegram:")
}

/// Returns whether a response should be routed to CLI.
fn should_send_cli_response(channel_id: &ChannelId) -> bool {
    channel_id.as_str().starts_with("cli:")
}

fn parse_worker_report(event: &ChannelEvent) -> Option<WorkerReport> {
    let metadata = event.metadata.clone()?;
    let report: WorkerReport = serde_json::from_value(metadata).ok()?;
    if report.kind == WORKER_REPORT_KIND {
        Some(report)
    } else {
        None
    }
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

    #[test]
    fn parse_repl_input_classifies_lines() {
        assert_eq!(parse_repl_input("   "), ReplInput::Empty);
        assert_eq!(parse_repl_input("/quit"), ReplInput::Exit);
        assert_eq!(parse_repl_input("/exit"), ReplInput::Exit);
        assert_eq!(
            parse_repl_input("  hello world  "),
            ReplInput::Message("hello world".to_string())
        );
    }

    #[test]
    fn should_send_mobile_response_checks_prefix() {
        assert!(should_send_mobile_response(&ChannelId::from(
            "mobile:dev1:req1"
        )));
        assert!(!should_send_mobile_response(&ChannelId::from(
            "telegram:123"
        )));
        assert!(!should_send_mobile_response(&ChannelId::from("cli:local")));
    }

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
    fn parse_worker_report_reads_tagged_metadata() {
        let report = WorkerReport::new(
            "call-1",
            "worker-a",
            "alpine:3.20",
            "echo hi",
            proto::WorkerOutput {
                exit_code: 0,
                stdout: "hi
"
                .to_string(),
                stderr: "".to_string(),
                output: "stdout:
hi

exit_code: 0"
                    .to_string(),
            },
        );
        let mut event = ChannelEvent::new(
            ChannelId::from("cli:local"),
            SessionId::from("s1"),
            "worker report",
        );
        event.metadata = Some(serde_json::to_value(report.clone()).expect("serialize report"));

        let parsed = parse_worker_report(&event).expect("worker report should parse");
        assert_eq!(parsed.kind, WORKER_REPORT_KIND);
        assert_eq!(parsed.call_id, report.call_id);
        assert_eq!(parsed.worker_id, report.worker_id);
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
}
