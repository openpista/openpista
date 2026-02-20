//! Per-connection QUIC session handling.

use proto::{ChannelEvent, GatewayError};
use quinn::Connection;
use tracing::{debug, error, info};

use crate::server::AgentHandler;

const MAX_MESSAGE_LEN: usize = 1_048_576;

/// Manages a single QUIC connection as an agent session
pub struct AgentSession {
    conn: Connection,
    handler: AgentHandler,
}

impl AgentSession {
    /// Creates a session wrapper for a QUIC connection and agent handler.
    pub fn new(conn: Connection, handler: AgentHandler) -> Self {
        Self { conn, handler }
    }

    /// Run the session: receive stream → process → send response
    pub async fn run(self) -> Result<(), GatewayError> {
        let remote = self.conn.remote_address();
        info!("AgentSession started for {remote}");

        loop {
            match self.conn.accept_bi().await {
                Ok((mut send, mut recv)) => {
                    let handler = self.handler.clone();
                    tokio::spawn(async move {
                        match handle_stream(&mut recv, &mut send, handler).await {
                            Ok(_) => debug!("Stream handled successfully"),
                            Err(e) => error!("Stream error: {e}"),
                        }
                    });
                }
                Err(quinn::ConnectionError::ApplicationClosed(_)) => {
                    info!("Connection closed by application from {remote}");
                    break;
                }
                Err(e) => {
                    error!("Connection error from {remote}: {e}");
                    return Err(GatewayError::Connection(e.to_string()));
                }
            }
        }

        Ok(())
    }
}

async fn handle_stream(
    recv: &mut quinn::RecvStream,
    send: &mut quinn::SendStream,
    handler: AgentHandler,
) -> Result<(), GatewayError> {
    // Read length-prefixed JSON
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| GatewayError::Connection(e.to_string()))?;
    let len = parse_message_len(len_buf);
    ensure_message_len(len)?;

    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| GatewayError::Connection(e.to_string()))?;

    let event = deserialize_event(&buf)?;

    debug!("Received event from channel: {}", event.channel_id);

    // Dispatch to handler
    let response_text = resolve_response_text(handler(event).await);

    // Send response
    let (resp_len, resp_bytes) = encode_response_payload(&response_text);
    send.write_all(&resp_len)
        .await
        .map_err(|e| GatewayError::Connection(e.to_string()))?;
    send.write_all(&resp_bytes)
        .await
        .map_err(|e| GatewayError::Connection(e.to_string()))?;
    send.finish()
        .map_err(|e| GatewayError::Connection(e.to_string()))?;

    Ok(())
}

/// Parses a big-endian 4-byte message length prefix.
fn parse_message_len(len_buf: [u8; 4]) -> usize {
    u32::from_be_bytes(len_buf) as usize
}

/// Validates inbound payload size against the maximum frame size.
fn ensure_message_len(len: usize) -> Result<(), GatewayError> {
    if len > MAX_MESSAGE_LEN {
        return Err(GatewayError::Connection("Message too large".into()));
    }
    Ok(())
}

/// Deserializes a JSON payload into a [`ChannelEvent`].
fn deserialize_event(buf: &[u8]) -> Result<ChannelEvent, GatewayError> {
    serde_json::from_slice(buf)
        .map_err(|e| GatewayError::Connection(format!("Deserialize error: {e}")))
}

/// Chooses the response body, falling back to `"OK"` for empty handler output.
fn resolve_response_text(handler_result: Option<String>) -> String {
    handler_result.unwrap_or_else(|| "OK".to_string())
}

/// Encodes response text as length-prefixed bytes.
fn encode_response_payload(response_text: &str) -> ([u8; 4], Vec<u8>) {
    let bytes = response_text.as_bytes().to_vec();
    ((bytes.len() as u32).to_be_bytes(), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_message_len_decodes_big_endian_u32() {
        let len = parse_message_len([0, 0, 1, 44]);
        assert_eq!(len, 300);
    }

    #[test]
    fn ensure_message_len_rejects_too_large_input() {
        assert!(ensure_message_len(MAX_MESSAGE_LEN).is_ok());
        let err = ensure_message_len(MAX_MESSAGE_LEN + 1).expect_err("len should be rejected");
        assert!(err.to_string().contains("Message too large"));
    }

    #[test]
    fn deserialize_event_parses_valid_json() {
        let event = ChannelEvent::new(
            proto::ChannelId::from("cli:local"),
            proto::SessionId::from("s1"),
            "hello",
        );
        let bytes = serde_json::to_vec(&event).expect("serialize");
        let parsed = deserialize_event(&bytes).expect("deserialize");
        assert_eq!(parsed.channel_id.as_str(), "cli:local");
        assert_eq!(parsed.session_id.as_str(), "s1");
        assert_eq!(parsed.user_message, "hello");
    }

    #[test]
    fn deserialize_event_reports_invalid_json() {
        let err = deserialize_event(b"{not json").expect_err("invalid json should fail");
        assert!(err.to_string().contains("Deserialize error"));
    }

    #[test]
    fn resolve_response_text_uses_default_on_none() {
        assert_eq!(resolve_response_text(Some("done".to_string())), "done");
        assert_eq!(resolve_response_text(None), "OK");
    }

    #[test]
    fn encode_response_payload_builds_length_prefixed_payload() {
        let (len, bytes) = encode_response_payload("pong");
        assert_eq!(u32::from_be_bytes(len), 4);
        assert_eq!(bytes, b"pong");
    }
}
