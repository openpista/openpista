//! Local CLI channel adapter.

use async_trait::async_trait;
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};
use tracing::info;

use crate::adapter::ChannelAdapter;

/// CLI adapter — reads from stdin, writes to stdout
pub struct CliAdapter {
    channel_id: ChannelId,
    session_id: SessionId,
    stdout: Mutex<tokio::io::Stdout>,
}

impl CliAdapter {
    /// Creates a new CLI adapter with a random session id.
    pub fn new() -> Self {
        Self {
            channel_id: ChannelId::new("cli", "local"),
            session_id: SessionId::new(),
            stdout: Mutex::new(tokio::io::stdout()),
        }
    }

    /// Creates a new CLI adapter bound to a specific session id.
    pub fn with_session(session_id: SessionId) -> Self {
        Self {
            channel_id: ChannelId::new("cli", "local"),
            session_id,
            stdout: Mutex::new(tokio::io::stdout()),
        }
    }
}

impl Default for CliAdapter {
    /// Creates a default CLI adapter.
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelAdapter for CliAdapter {
    fn channel_id(&self) -> ChannelId {
        self.channel_id.clone()
    }

    async fn run(self, tx: mpsc::Sender<ChannelEvent>) -> Result<(), ChannelError> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin).lines();
        let mut stdout = tokio::io::stdout();

        info!("CLI adapter started (session: {})", self.session_id);

        stdout
            .write_all(b"openpistacrab> ")
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
        stdout
            .flush()
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;

        while let Ok(Some(line)) = reader.next_line().await {
            let Some(line) = normalize_input_line(&line) else {
                stdout
                    .write_all(b"openpistacrab> ")
                    .await
                    .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
                stdout
                    .flush()
                    .await
                    .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
                continue;
            };

            if is_quit_command(&line) {
                break;
            }

            let event = ChannelEvent::new(self.channel_id.clone(), self.session_id.clone(), line);

            tx.send(event)
                .await
                .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
        }

        info!("CLI adapter stopped");
        Ok(())
    }

    async fn send_response(&self, resp: AgentResponse) -> Result<(), ChannelError> {
        let mut stdout = self.stdout.lock().await;
        let output = format_prompted_response(&resp);
        stdout
            .write_all(output.as_bytes())
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
        stdout
            .flush()
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))
    }
}

/// Trims an input line and drops empty lines.
fn normalize_input_line(raw: &str) -> Option<String> {
    let line = raw.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

/// Returns true when input requests REPL termination.
fn is_quit_command(line: &str) -> bool {
    line == "/quit" || line == "/exit"
}

/// Formats outbound text with prompt suffix for CLI UX.
fn format_prompted_response(resp: &AgentResponse) -> String {
    let prefix = if resp.is_error { "Error: " } else { "" };
    format!("\n{}{}\n\nopenpistacrab> ", prefix, resp.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_set_channel_and_session() {
        let adapter = CliAdapter::new();
        assert_eq!(adapter.channel_id.as_str(), "cli:local");
        assert!(!adapter.session_id.as_str().is_empty());

        let session = SessionId::from("fixed");
        let adapter = CliAdapter::with_session(session.clone());
        assert_eq!(adapter.channel_id.as_str(), "cli:local");
        assert_eq!(adapter.session_id, session);
    }

    #[test]
    fn normalize_input_line_trims_and_filters_empty() {
        assert_eq!(normalize_input_line("  hello "), Some("hello".to_string()));
        assert_eq!(normalize_input_line("   "), None);
        assert_eq!(normalize_input_line(""), None);
    }

    #[test]
    fn quit_commands_are_detected() {
        assert!(is_quit_command("/quit"));
        assert!(is_quit_command("/exit"));
        assert!(!is_quit_command("/help"));
    }

    #[test]
    fn prompted_response_formats_success_and_error() {
        let ok = AgentResponse::new(ChannelId::from("cli:local"), SessionId::from("s1"), "done");
        let out = format_prompted_response(&ok);
        assert!(out.contains("\ndone\n"));
        assert!(out.ends_with("openpistacrab> "));

        let err = AgentResponse::error(ChannelId::from("cli:local"), SessionId::from("s1"), "boom");
        let out = format_prompted_response(&err);
        assert!(out.contains("Error: boom"));
    }
}
