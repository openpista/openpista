//! WhatsApp channel subcommand handlers.

#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use agent::AutoApproveHandler;
#[cfg(not(test))]
use proto::{ChannelId, SessionId};
#[cfg(not(test))]
use skills::SkillLoader;

#[cfg(not(test))]
use crate::config::Config;
#[cfg(not(test))]
use crate::startup::build_runtime;

#[cfg(not(test))]
pub(crate) fn prompt_whatsapp_model_warning(config: &Config) -> anyhow::Result<bool> {
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

/// Drains stderr from a bridge subprocess in a background task to prevent pipe deadlock.
#[cfg(not(test))]
pub(crate) fn spawn_bridge_stderr_drain(stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(_)) = lines.next_line().await {}
    });
}

#[cfg(not(test))]
/// Non-TUI WhatsApp setup: check prerequisites, install bridge deps, spawn bridge,
/// display QR in terminal, and save config on successful connection.
pub(crate) async fn cmd_whatsapp(mut config: Config) -> anyhow::Result<()> {
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
    let bridge_path = config.channels.whatsapp.effective_bridge_path().to_owned();
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
    spawn_bridge_stderr_drain(stderr);

    // 4. Read bridge events
    println!("Waiting for QR code... (scan with your phone)");
    println!();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if let Ok(event) = serde_json::from_str::<channels::whatsapp::BridgeEvent>(&line) {
                    match event {
                        channels::whatsapp::BridgeEvent::Qr { data } => {
                            if let Some(qr_text) = crate::tui::event::render_qr_text(&data) {
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
pub(crate) async fn cmd_whatsapp_status(config: Config) -> anyhow::Result<()> {
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
            && let Some(me_id) = json
                .get("me")
                .and_then(|me| me.get("id"))
                .and_then(|id| id.as_str())
        {
            let phone = me_id.split(':').next().unwrap_or(me_id);
            println!("Phone:       {phone}");
            println!("Link:        https://wa.me/{phone}");
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
pub(crate) async fn cmd_whatsapp_start(config: Config) -> anyhow::Result<()> {
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

    let bridge_path = config.channels.whatsapp.effective_bridge_path().to_owned();

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
                                    if let Some(qr_text) = crate::tui::event::render_qr_text(&data) {
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
                                            Ok((text, _usage)) => text,
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
pub(crate) async fn cmd_whatsapp_send(
    config: Config,
    number: String,
    message: String,
) -> anyhow::Result<()> {
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

    let bridge_path = config.channels.whatsapp.effective_bridge_path().to_owned();

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
    spawn_bridge_stderr_drain(stderr);

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
