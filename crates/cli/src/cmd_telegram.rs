//! Telegram channel subcommand handlers.

#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use agent::AutoApproveHandler;
#[cfg(not(test))]
use channels::{ChannelAdapter, TelegramAdapter};
#[cfg(not(test))]
use proto::{AgentResponse, ChannelEvent};
#[cfg(not(test))]
use skills::SkillLoader;
#[cfg(not(test))]
use tracing::error;

#[cfg(not(test))]
use crate::config::Config;
#[cfg(not(test))]
use crate::startup::build_runtime;

/// Validates that a Telegram bot token matches the expected `NUMBERS:STRING` format.
pub(crate) fn is_valid_telegram_token(token: &str) -> bool {
    let mut parts = token.splitn(2, ':');
    let numeric = parts
        .next()
        .map(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty());
    let rest = parts.next().map(|p| !p.is_empty());
    matches!((numeric, rest), (Some(true), Some(true)))
}

#[cfg(not(test))]
/// `openpista telegram status` — prints current Telegram channel configuration.
pub(crate) fn cmd_telegram_status(config: &Config) -> anyhow::Result<()> {
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
pub(crate) async fn cmd_telegram_setup(
    mut config: Config,
    token: Option<String>,
) -> anyhow::Result<()> {
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
pub(crate) async fn cmd_telegram_start(
    config: Config,
    token: Option<String>,
) -> anyhow::Result<()> {
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
            println!("Telegram Bot Setup");
            println!("==================");
            println!();
            println!("No token configured. Follow these steps to create a bot first:");
            println!();
            println!("1. Open the Telegram app, search for @BotFather, and start a chat");
            println!("2. Send /newbot");
            println!("3. Enter a name for your bot (e.g. MyPistaBot)");
            println!("4. Enter a username for your bot (e.g. mypista_bot)  ← Name ending in _bot");
            println!("5. BotFather will provide your token:");
            println!("     123456789:AABBccDDeeFFggHH...");
            println!();
            println!("After receiving the token, run:");
            println!("  openpista telegram setup --token YOUR_TOKEN");
            println!("  openpista telegram start");
            println!();
            return Ok(());
        }
    };

    if !is_valid_telegram_token(&token) {
        anyhow::bail!(
            "Invalid token format '{}'. Expected NUMBERS:STRING (e.g. 123456:ABC...)",
            token
        );
    }

    let effective_model = config.agent.effective_model().to_string();
    if effective_model.is_empty() || config.agent.model.is_empty() {
        println!(
            "\u{26a0}  No model configured. Telegram needs an LLM model to respond to messages."
        );
        println!(
            "  Current: provider={}, model={}",
            config.agent.provider.name(),
            if effective_model.is_empty() {
                "(none)"
            } else {
                &effective_model
            }
        );
        println!();
        println!("  Run `openpista model select` to choose a model first.");
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
            return Ok(());
        }
        println!();
    }
    println!("Telegram Bot Server");
    println!("===================");
    println!();
    println!("Provider: {}", config.agent.provider.name());
    println!("Model   : {effective_model}");
    println!();
    println!("Starting agent runtime...");

    let runtime = build_runtime(&config, Arc::new(AutoApproveHandler)).await?;
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
                let resp = crate::build_agent_response(&event, result);
                let _ = tx.send(resp).await;
            });
        }
    });

    // Response dispatch: send LLM reply back to user
    tokio::spawn(async move {
        while let Some(resp) = resp_rx.recv().await {
            if crate::should_send_telegram_response(&resp.channel_id)
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
