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
use proto::{AgentResponse, ChannelEvent, ChannelId, SessionId, WORKER_REPORT_KIND, WorkerReport};

#[cfg(not(test))]
use std::net::SocketAddr;
#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use crate::auth_picker::{AuthLoginIntent, AuthMethodChoice};
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

    /// Browse model catalog entries
    Models {
        #[command(subcommand)]
        command: ModelsCommands,
    },

    /// Manage provider credentials via OAuth 2.0 PKCE browser login
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
}

/// `models` sub-subcommands.
#[derive(Subcommand)]
enum ModelsCommands {
    /// List recommended coding models
    List,
}

/// `auth` sub-subcommands.
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
        Commands::Models { command } => cmd_models(command).await,
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

    tui::run_tui(
        runtime,
        skill_loader,
        channel_id,
        session_id,
        model_name,
        config.clone(),
    )
    .await
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
        warn!("No API key configured. Set openpista_API_KEY or OPENAI_API_KEY.");
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
    info!("Starting openpista daemon");

    let report_host = config.gateway.report_host.as_deref().unwrap_or("127.0.0.1");
    let report_addr = format!("{report_host}:{}", config.gateway.port);
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
    info!("openpista stopped");
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
async fn cmd_models(command: ModelsCommands) -> anyhow::Result<()> {
    match command {
        ModelsCommands::List => {
            let catalog = model_catalog::load_opencode_catalog(false).await;
            let summary = model_catalog::model_summary(&catalog.entries, "", false);
            let sections = model_catalog::model_sections(&catalog.entries, "", false);

            println!(
                "Models | provider:{} | total:{} | matched:{} | recommended:{} | available:{}",
                catalog.provider,
                summary.total,
                summary.matched,
                summary.recommended,
                summary.available
            );
            println!("{}", catalog.sync_status);
            println!();

            print_model_section("Recommended + Available", &sections.recommended_available);
            print_model_section(
                "Recommended + Unavailable",
                &sections.recommended_unavailable,
            );
        }
    }

    Ok(())
}

#[cfg(not(test))]
fn print_model_section(title: &str, entries: &[model_catalog::ModelCatalogEntry]) {
    println!("{title} ({})", entries.len());
    for entry in entries {
        println!(
            "- {}  [status:{}]  [available:{}]  [source:{}]",
            entry.id,
            entry.status.as_str(),
            if entry.available { "yes" } else { "no" },
            entry.source.as_str()
        );
    }
    println!();
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
