//! WhatsApp Web multi-device channel adapter (Baileys bridge subprocess).
//!
//! Communicates with a Node.js bridge process over JSON lines on stdin/stdout.
//! Users pair by scanning a QR code — no API keys or webhooks needed.

use async_trait::async_trait;
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::adapter::ChannelAdapter;

// ─── Bridge protocol types ─────────────────────────────────

/// Commands sent from Rust → Bridge (JSON lines on stdin).
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BridgeCommand {
    /// Send a text message to a WhatsApp number.
    Send { to: String, text: String },
    /// Gracefully disconnect the bridge.
    Disconnect,
    /// Graceful shutdown — closes WebSocket without logging out.
    Shutdown,
}

/// Events received from Bridge → Rust (JSON lines on stdout).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BridgeEvent {
    /// QR code data for pairing.
    Qr { data: String },
    /// Successfully connected / paired.
    Connected { phone: String, name: Option<String> },
    /// Incoming text message.
    Message {
        /// Phone number of the sender.
        from: String,
        /// Plain-text message body.
        text: String,
        #[allow(dead_code)]
        /// Unix timestamp of the message, if provided by the bridge.
        timestamp: Option<u64>,
        #[serde(default)]
        #[serde(rename = "selfChat")]
        #[allow(dead_code)]
        /// Whether the message was sent by the authenticated user (self-chat).
        self_chat: bool,
    },
    /// Disconnected from WhatsApp Web.
    Disconnected {
        /// Human-readable reason for the disconnect, if available.
        reason: Option<String>,
    },
    /// Bridge-level error.
    Error {
        /// Error message from the bridge.
        message: String,
    },
}

// ─── Adapter config ────────────────────────────────────────

/// Configuration for the WhatsApp adapter.
///
/// Mirrors `WhatsAppConfig` from `crates/cli/src/config.rs` to avoid a
/// reverse dependency from channels → cli.
#[derive(Debug, Clone)]
pub struct WhatsAppAdapterConfig {
    /// Directory for WhatsApp Web session auth state.
    pub session_dir: String,
    /// Path to the Node.js bridge script. `None` = bundled default.
    pub bridge_path: Option<String>,
}

// ─── Adapter ───────────────────────────────────────────────

/// WhatsApp Web multi-device adapter.
///
/// Spawns a Node.js bridge subprocess that uses Baileys to connect to
/// WhatsApp Web. Communication is via JSON lines over stdin/stdout.
pub struct WhatsAppAdapter {
    config: WhatsAppAdapterConfig,
    /// Channel for sending commands to the bridge stdin writer task.
    cmd_tx: mpsc::Sender<BridgeCommand>,
    /// Receiver end — moved into `run()`.
    cmd_rx: Option<mpsc::Receiver<BridgeCommand>>,
    /// Channel for forwarding QR codes to the TUI.
    #[allow(dead_code)]
    qr_tx: mpsc::Sender<String>,
    #[allow(dead_code)]
    resp_tx: mpsc::Sender<AgentResponse>,
}

impl WhatsAppAdapter {
    /// Creates a new WhatsApp adapter.
    ///
    /// * `config` — session directory and bridge path
    /// * `resp_tx` — channel for sending agent responses back
    /// * `qr_tx` — channel for forwarding QR code data to the TUI
    pub fn new(
        config: WhatsAppAdapterConfig,
        resp_tx: mpsc::Sender<AgentResponse>,
        qr_tx: mpsc::Sender<String>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        Self {
            config,
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            qr_tx,
            resp_tx,
        }
    }

    /// Returns a sender that can be used to send commands to the bridge.
    pub fn command_sender(&self) -> mpsc::Sender<BridgeCommand> {
        self.cmd_tx.clone()
    }

    /// Creates a stable session id for a WhatsApp phone number.
    fn make_session_id(phone: &str) -> SessionId {
        SessionId::from(format!("whatsapp:{phone}"))
    }

    /// Resolves the bridge script path.
    fn bridge_script(&self) -> String {
        self.config
            .bridge_path
            .clone()
            .unwrap_or_else(|| "whatsapp-bridge/index.js".to_string())
    }
}

// ─── ChannelAdapter impl ───────────────────────────────────

/// [`ChannelAdapter`] implementation for the WhatsApp bridge adapter.
#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    /// Returns the stable [`ChannelId`] identifying this adapter (`whatsapp:bridge`).
    fn channel_id(&self) -> ChannelId {
        ChannelId::new("whatsapp", "bridge")
    }

    /// Starts the WhatsApp bridge subprocess and processes events until the bridge exits.
    async fn run(mut self, tx: mpsc::Sender<ChannelEvent>) -> Result<(), ChannelError> {
        let bridge_script = self.bridge_script();
        info!(
            bridge = %bridge_script,
            session_dir = %self.config.session_dir,
            "WhatsApp bridge adapter starting"
        );

        // Ensure session directory exists
        tokio::fs::create_dir_all(&self.config.session_dir)
            .await
            .map_err(|e| {
                ChannelError::ConnectionFailed(format!("failed to create session dir: {e}"))
            })?;

        // Spawn the Node.js bridge subprocess
        let mut child = TokioCommand::new("node")
            .arg(&bridge_script)
            .arg(&self.config.session_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ChannelError::ConnectionFailed(format!("failed to spawn bridge: {e}")))?;

        let stdin = child.stdin.take().ok_or_else(|| {
            ChannelError::ConnectionFailed("bridge stdin not available".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ChannelError::ConnectionFailed("bridge stdout not available".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ChannelError::ConnectionFailed("bridge stderr not available".to_string())
        })?;

        // Take the command receiver (only available once)
        let mut cmd_rx = self.cmd_rx.take().ok_or_else(|| {
            ChannelError::ConnectionFailed("command receiver already consumed".to_string())
        })?;

        let qr_tx = self.qr_tx.clone();

        // Task: read bridge stdout (JSON lines → events)
        let event_tx = tx.clone();
        let stdout_task = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                match serde_json::from_str::<BridgeEvent>(&line) {
                    Ok(BridgeEvent::Qr { data }) => {
                        info!("WhatsApp QR code received");
                        if let Err(e) = qr_tx.send(data).await {
                            warn!("Failed to forward QR code: {e}");
                        }
                    }
                    Ok(BridgeEvent::Connected { phone, name }) => {
                        info!(
                            phone = %phone,
                            name = ?name,
                            "WhatsApp Web connected"
                        );
                        // Send a synthetic event so the gateway knows we're connected
                        let channel_id = ChannelId::new("whatsapp", &phone);
                        let session_id = WhatsAppAdapter::make_session_id(&phone);
                        let event = ChannelEvent::new(
                            channel_id,
                            session_id,
                            format!(
                                "[WhatsApp connected: {} ({})]",
                                phone,
                                name.unwrap_or_default()
                            ),
                        );
                        let _ = event_tx.send(event).await;
                    }
                    Ok(BridgeEvent::Message { from, text, .. }) => {
                        debug!(from = %from, "WhatsApp message received");
                        let channel_id = ChannelId::new("whatsapp", &from);
                        let session_id = WhatsAppAdapter::make_session_id(&from);
                        let event = ChannelEvent::new(channel_id, session_id, text);
                        if let Err(e) = event_tx.send(event).await {
                            error!("Failed to forward WhatsApp event: {e}");
                        }
                    }
                    Ok(BridgeEvent::Disconnected { reason }) => {
                        let reason_str = reason.as_deref().unwrap_or("unknown");
                        if reason_str == "logged out" {
                            warn!("WhatsApp session logged out — adapter stopping");
                            break;
                        }
                        warn!(reason = %reason_str, "WhatsApp transient disconnect (bridge reconnecting)");
                    }
                    Ok(BridgeEvent::Error { message }) => {
                        error!(message = %message, "WhatsApp bridge error");
                    }
                    Err(e) => {
                        warn!(line = %line, error = %e, "Failed to parse bridge event");
                    }
                }
            }
        });

        // Task: read bridge stderr (log to tracing)
        let stderr_task = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(target: "whatsapp_bridge", "{}", line);
            }
        });

        // Task: write commands to bridge stdin
        let stdin_task = tokio::spawn(async move {
            let mut writer = stdin;
            while let Some(cmd) = cmd_rx.recv().await {
                match serde_json::to_string(&cmd) {
                    Ok(json) => {
                        let line = format!("{json}\n");
                        if let Err(e) = writer.write_all(line.as_bytes()).await {
                            error!("Failed to write to bridge stdin: {e}");
                            break;
                        }
                        if let Err(e) = writer.flush().await {
                            error!("Failed to flush bridge stdin: {e}");
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to serialize bridge command: {e}");
                    }
                }
            }
        });

        // Wait for stdout reader to finish (bridge exited or disconnected)
        let _ = stdout_task.await;
        stderr_task.abort();
        stdin_task.abort();

        // Wait for the child process to exit
        match child.wait().await {
            Ok(status) => {
                info!(status = %status, "WhatsApp bridge process exited");
            }
            Err(e) => {
                warn!("Failed to wait for bridge process: {e}");
            }
        }

        info!("WhatsApp adapter stopped");
        Ok(())
    }

    /// Sends an [`AgentResponse`] to a WhatsApp phone number via the bridge.
    async fn send_response(&self, resp: AgentResponse) -> Result<(), ChannelError> {
        let phone = parse_phone_from_channel_id(resp.channel_id.as_str())?;
        let text = format_response_text(&resp);

        let cmd = BridgeCommand::Send { to: phone, text };
        self.cmd_tx.send(cmd).await.map_err(|e| {
            ChannelError::SendFailed(format!("failed to send command to bridge: {e}"))
        })?;

        Ok(())
    }
}

// ─── Helpers ───────────────────────────────────────────────

/// Parses a phone number from `whatsapp:<phone>` or raw string.
fn parse_phone_from_channel_id(channel_str: &str) -> Result<String, ChannelError> {
    let phone = channel_str.strip_prefix("whatsapp:").unwrap_or(channel_str);
    if phone.is_empty() {
        Err(ChannelError::SendFailed(
            "Empty phone number in channel id".to_string(),
        ))
    } else {
        Ok(phone.to_string())
    }
}

/// Formats response text with error marker when needed.
fn format_response_text(resp: &AgentResponse) -> String {
    if resp.is_error {
        format!("❌ Error: {}", resp.content)
    } else {
        resp.content.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_session_id_uses_whatsapp_prefix() {
        let sid = WhatsAppAdapter::make_session_id("15551234567");
        assert_eq!(sid.as_str(), "whatsapp:15551234567");
    }

    #[test]
    fn parse_phone_handles_prefixed_and_raw() {
        assert_eq!(
            parse_phone_from_channel_id("whatsapp:15551234567").unwrap(),
            "15551234567"
        );
        assert_eq!(
            parse_phone_from_channel_id("15551234567").unwrap(),
            "15551234567"
        );
    }

    #[test]
    fn parse_phone_rejects_empty() {
        assert!(parse_phone_from_channel_id("whatsapp:").is_err());
    }

    #[test]
    fn format_response_text_marks_errors() {
        let ok = AgentResponse::new(ChannelId::from("whatsapp:1"), SessionId::from("s1"), "ok");
        assert_eq!(format_response_text(&ok), "ok");

        let err =
            AgentResponse::error(ChannelId::from("whatsapp:1"), SessionId::from("s1"), "boom");
        assert!(format_response_text(&err).starts_with("❌ Error: "));
    }

    #[test]
    fn bridge_command_serializes_correctly() {
        let cmd = BridgeCommand::Send {
            to: "15551234567".to_string(),
            text: "Hello".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""type":"send"#));
        assert!(json.contains(r#""to":"15551234567"#));
        assert!(json.contains(r#""text":"Hello"#));
    }

    #[test]
    fn bridge_command_disconnect_serializes() {
        let cmd = BridgeCommand::Disconnect;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""type":"disconnect"#));
    }

    #[test]
    fn bridge_command_shutdown_serializes() {
        let cmd = BridgeCommand::Shutdown;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""type":"shutdown"#));
    }
    #[test]
    fn bridge_event_qr_deserializes() {
        let json = r#"{"type":"qr","data":"2@ABC123"}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, BridgeEvent::Qr { data } if data == "2@ABC123"));
    }

    #[test]
    fn bridge_event_connected_deserializes() {
        let json = r#"{"type":"connected","phone":"15551234567","name":"John"}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(
            matches!(event, BridgeEvent::Connected { phone, name } if phone == "15551234567" && name.as_deref() == Some("John"))
        );
    }

    #[test]
    fn bridge_event_message_deserializes() {
        let json =
            r#"{"type":"message","from":"15551234567","text":"Hello!","timestamp":1234567890}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(
            matches!(event, BridgeEvent::Message { from, text, .. } if from == "15551234567" && text == "Hello!")
        );
    }

    #[test]
    fn bridge_event_message_self_chat_deserializes() {
        let json = r#"{"type":"message","from":"15551234567","text":"Hi","timestamp":1234567890,"selfChat":true}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(
            matches!(event, BridgeEvent::Message { from, text, self_chat, .. } if from == "15551234567" && text == "Hi" && self_chat)
        );
    }

    #[test]
    fn bridge_event_disconnected_deserializes() {
        let json = r#"{"type":"disconnected","reason":"logged out"}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(
            matches!(event, BridgeEvent::Disconnected { reason } if reason.as_deref() == Some("logged out"))
        );
    }

    #[test]
    fn bridge_event_error_deserializes() {
        let json = r#"{"type":"error","message":"connection failed"}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, BridgeEvent::Error { message } if message == "connection failed"));
    }

    #[test]
    fn adapter_constructor_and_channel_id() {
        let config = WhatsAppAdapterConfig {
            session_dir: "/tmp/wa-session".to_string(),
            bridge_path: None,
        };
        let (resp_tx, _resp_rx) = mpsc::channel(1);
        let (qr_tx, _qr_rx) = mpsc::channel(1);
        let adapter = WhatsAppAdapter::new(config, resp_tx, qr_tx);
        assert_eq!(adapter.channel_id().as_str(), "whatsapp:bridge");
    }

    #[test]
    fn bridge_script_defaults_to_bundled_path() {
        let config = WhatsAppAdapterConfig {
            session_dir: "/tmp".to_string(),
            bridge_path: None,
        };
        let (resp_tx, _resp_rx) = mpsc::channel(1);
        let (qr_tx, _qr_rx) = mpsc::channel(1);
        let adapter = WhatsAppAdapter::new(config, resp_tx, qr_tx);
        assert_eq!(adapter.bridge_script(), "whatsapp-bridge/index.js");
    }

    #[test]
    fn bridge_script_uses_custom_path_when_set() {
        let config = WhatsAppAdapterConfig {
            session_dir: "/tmp".to_string(),
            bridge_path: Some("/custom/bridge.js".to_string()),
        };
        let (resp_tx, _resp_rx) = mpsc::channel(1);
        let (qr_tx, _qr_rx) = mpsc::channel(1);
        let adapter = WhatsAppAdapter::new(config, resp_tx, qr_tx);
        assert_eq!(adapter.bridge_script(), "/custom/bridge.js");
    }

    #[test]
    fn command_sender_returns_working_sender() {
        let config = WhatsAppAdapterConfig {
            session_dir: "/tmp".to_string(),
            bridge_path: None,
        };
        let (resp_tx, _resp_rx) = mpsc::channel(1);
        let (qr_tx, _qr_rx) = mpsc::channel(1);
        let adapter = WhatsAppAdapter::new(config, resp_tx, qr_tx);
        let _sender = adapter.command_sender();
    }

    #[test]
    fn bridge_event_disconnected_without_reason() {
        let json = r#"{"type":"disconnected"}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, BridgeEvent::Disconnected { reason } if reason.is_none()));
    }

    #[test]
    fn bridge_event_connected_without_name() {
        let json = r#"{"type":"connected","phone":"123"}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(
            matches!(event, BridgeEvent::Connected { phone, name } if phone == "123" && name.is_none())
        );
    }

    #[test]
    fn bridge_event_message_without_timestamp() {
        let json = r#"{"type":"message","from":"123","text":"hi"}"#;
        let event: BridgeEvent = serde_json::from_str(json).unwrap();
        assert!(
            matches!(event, BridgeEvent::Message { from, text, timestamp, .. } if from == "123" && text == "hi" && timestamp.is_none())
        );
    }

    #[test]
    fn parse_phone_handles_raw_number() {
        assert_eq!(parse_phone_from_channel_id("5551234").unwrap(), "5551234");
    }

    #[test]
    fn parse_phone_rejects_bare_empty() {
        assert!(parse_phone_from_channel_id("").is_err());
    }
}
