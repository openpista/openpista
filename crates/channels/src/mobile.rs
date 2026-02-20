//! QUIC-based mobile channel adapter.
//!
//! Mobile clients connect over QUIC, authenticate with a bearer token, then
//! exchange messages in a simple length-prefixed JSON protocol.
//!
//! **Identity scheme**:
//! - `channel_id` = `"mobile:<device_id>:<request_uuid>"` — unique per request,
//!   used as the DashMap key to route the oneshot response back to the waiting
//!   bi-stream.
//! - `session_id` = `"mobile:<device_id>"` — stable per device so conversation
//!   history is preserved across requests.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::adapter::ChannelAdapter;

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum accepted frame size (1 MiB), matching the gateway's `MAX_MESSAGE_LEN`.
const MAX_FRAME_LEN: usize = 1_048_576;

/// Seconds to wait for an agent response before sending an error back to the client.
const RESPONSE_TIMEOUT_SECS: u64 = 120;

// ─── Wire protocol ───────────────────────────────────────────────────────────

/// Messages the client sends to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Auth(AuthRequest),
    Message(UserMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthRequest {
    token: String,
    device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserMessage {
    text: String,
}

/// Messages the server sends to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    AuthOk(AuthOkPayload),
    AuthError(ErrorPayload),
    Response(ResponsePayload),
    Error(ErrorPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthOkPayload {
    session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResponsePayload {
    content: String,
    is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ErrorPayload {
    message: String,
}

// ─── Shared interior ─────────────────────────────────────────────────────────

/// State shared between the `run` instance (which manages QUIC connections) and
/// the `send_response` instance (called by the response-forwarder in `cmd_start`).
struct Inner {
    api_token: String,
    bind_addr: SocketAddr,
    /// Maps compound channel_id `"mobile:<device_id>:<req_uuid>"` to a oneshot
    /// sender that delivers the `AgentResponse` to the waiting bi-stream handler.
    pending: DashMap<String, oneshot::Sender<AgentResponse>>,
}

// ─── MobileAdapter ───────────────────────────────────────────────────────────

/// QUIC-based channel adapter for mobile clients.
pub struct MobileAdapter {
    inner: Arc<Inner>,
}

impl MobileAdapter {
    /// Creates a new adapter that will listen on `bind_addr` and require
    /// `api_token` on every new connection.
    pub fn new(bind_addr: SocketAddr, api_token: String) -> Self {
        Self {
            inner: Arc::new(Inner {
                api_token,
                bind_addr,
                pending: DashMap::new(),
            }),
        }
    }

    /// Returns a second handle that shares the same pending-response map as
    /// `self`.  Pass the original to `run()` and keep this handle for
    /// `send_response()` calls from the response forwarder.
    pub fn response_handle(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[async_trait]
impl ChannelAdapter for MobileAdapter {
    fn channel_id(&self) -> ChannelId {
        ChannelId::new("mobile", "quic")
    }

    async fn run(self, tx: mpsc::Sender<ChannelEvent>) -> Result<(), ChannelError> {
        info!("Mobile QUIC adapter listening on {}", self.inner.bind_addr);

        // Build a self-signed TLS certificate (same approach as gateway::server).
        let (cert_der, key_der) =
            generate_self_signed().map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?;

        let mut tls_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?;
        tls_config.max_early_data_size = u32::MAX;

        let quic_server_config = quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
                .map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?,
        ));

        let endpoint = quinn::Endpoint::server(quic_server_config, self.inner.bind_addr)
            .map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?;

        loop {
            match endpoint.accept().await {
                Some(incoming) => {
                    let inner = Arc::clone(&self.inner);
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        match incoming.await {
                            Ok(conn) => {
                                let remote = conn.remote_address();
                                info!("Mobile: new QUIC connection from {remote}");
                                if let Err(e) = handle_connection(conn, inner, tx).await {
                                    warn!("Mobile: connection error from {remote}: {e}");
                                }
                            }
                            Err(e) => {
                                warn!("Mobile: failed to accept connection: {e}");
                            }
                        }
                    });
                }
                None => {
                    info!("Mobile: QUIC endpoint closed");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn send_response(&self, resp: AgentResponse) -> Result<(), ChannelError> {
        let key = resp.channel_id.as_str().to_string();

        match self.inner.pending.remove(&key) {
            Some((_, sender)) => {
                // Receiver may have dropped (timeout / disconnect); that's fine.
                let _ = sender.send(resp);
            }
            None => {
                warn!(
                    "Mobile: no pending request for channel_id '{key}' (timed out or already responded)"
                );
            }
        }

        Ok(())
    }
}

// ─── Connection handler ───────────────────────────────────────────────────────

/// Handles one authenticated QUIC connection: auth handshake on the first
/// bi-stream, then message/response on each subsequent bi-stream.
async fn handle_connection(
    conn: quinn::Connection,
    inner: Arc<Inner>,
    tx: mpsc::Sender<ChannelEvent>,
) -> Result<(), ChannelError> {
    // ── Auth bi-stream ──────────────────────────────────────────────────────
    let (mut auth_send, mut auth_recv) = conn
        .accept_bi()
        .await
        .map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?;

    let auth_buf = read_frame(&mut auth_recv).await?;
    let auth_msg = parse_client_message(&auth_buf)?;

    let device_id = match auth_msg {
        ClientMessage::Auth(req) => match validate_auth(&req, &inner.api_token) {
            Ok(id) => {
                let ok = ServerMessage::AuthOk(AuthOkPayload {
                    session_id: make_session_id(&id).as_str().to_string(),
                });
                let payload = encode_server_message(&ok)?;
                write_frame(&mut auth_send, &payload).await?;
                auth_send
                    .finish()
                    .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
                id
            }
            Err(e) => {
                let err_msg = ServerMessage::AuthError(ErrorPayload {
                    message: e.to_string(),
                });
                let payload = encode_server_message(&err_msg)?;
                write_frame(&mut auth_send, &payload).await?;
                let _ = auth_send.finish();
                return Err(e);
            }
        },
        _ => {
            let err_msg = ServerMessage::AuthError(ErrorPayload {
                message: "Expected auth message first".into(),
            });
            let payload = encode_server_message(&err_msg)?;
            write_frame(&mut auth_send, &payload).await?;
            let _ = auth_send.finish();
            return Err(ChannelError::AuthFailed(
                "Expected auth message first".into(),
            ));
        }
    };

    info!("Mobile: device '{device_id}' authenticated");

    // ── Message bi-streams ─────────────────────────────────────────────────
    loop {
        match conn.accept_bi().await {
            Ok((msg_send, msg_recv)) => {
                let inner = Arc::clone(&inner);
                let tx = tx.clone();
                let device_id = device_id.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        handle_message_stream(msg_send, msg_recv, inner, tx, &device_id).await
                    {
                        warn!("Mobile: stream error for device '{device_id}': {e}");
                    }
                });
            }
            Err(quinn::ConnectionError::ApplicationClosed(_)) => {
                info!("Mobile: device '{device_id}' disconnected");
                break;
            }
            Err(e) => {
                error!("Mobile: connection error for device '{device_id}': {e}");
                break;
            }
        }
    }

    Ok(())
}

/// Handles one message bi-stream: read message → enqueue event → await response → write back.
async fn handle_message_stream(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    inner: Arc<Inner>,
    tx: mpsc::Sender<ChannelEvent>,
    device_id: &str,
) -> Result<(), ChannelError> {
    let buf = read_frame(&mut recv).await?;
    let msg = parse_client_message(&buf)?;

    let text = match msg {
        ClientMessage::Message(m) => m.text,
        _ => {
            let err = ServerMessage::Error(ErrorPayload {
                message: "Expected message, got auth frame".into(),
            });
            let payload = encode_server_message(&err)?;
            write_frame(&mut send, &payload).await?;
            let _ = send.finish();
            return Ok(());
        }
    };

    // Unique channel_id per request; stable session_id per device.
    let request_id = Uuid::new_v4().to_string();
    let channel_id = make_channel_id(device_id, &request_id);
    let session_id = make_session_id(device_id);

    let (resp_tx, resp_rx) = oneshot::channel::<AgentResponse>();
    inner
        .pending
        .insert(channel_id.as_str().to_string(), resp_tx);

    if tx
        .send(ChannelEvent::new(channel_id.clone(), session_id, text))
        .await
        .is_err()
    {
        inner.pending.remove(channel_id.as_str());
        let err = ServerMessage::Error(ErrorPayload {
            message: "Agent unavailable".into(),
        });
        let payload = encode_server_message(&err)?;
        write_frame(&mut send, &payload).await?;
        let _ = send.finish();
        return Ok(());
    }

    // Wait for the response-forwarder to call send_response().
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(RESPONSE_TIMEOUT_SECS),
        resp_rx,
    )
    .await;

    // Clean up any leftover pending entry (timeout path).
    inner.pending.remove(channel_id.as_str());

    let server_msg = match result {
        Ok(Ok(resp)) => ServerMessage::Response(ResponsePayload {
            content: resp.content,
            is_error: resp.is_error,
        }),
        Ok(Err(_)) => ServerMessage::Error(ErrorPayload {
            message: "Response channel closed".into(),
        }),
        Err(_) => ServerMessage::Error(ErrorPayload {
            message: "Request timed out".into(),
        }),
    };

    let payload = encode_server_message(&server_msg)?;
    write_frame(&mut send, &payload).await?;
    send.finish()
        .map_err(|e| ChannelError::SendFailed(e.to_string()))?;

    Ok(())
}

// ─── Pure helper functions ────────────────────────────────────────────────────

/// Validates the bearer token from an auth request.
/// Returns the `device_id` on success.
fn validate_auth(req: &AuthRequest, expected_token: &str) -> Result<String, ChannelError> {
    if req.token != expected_token {
        return Err(ChannelError::AuthFailed("Invalid API token".into()));
    }
    if req.device_id.is_empty() {
        return Err(ChannelError::AuthFailed(
            "device_id must not be empty".into(),
        ));
    }
    Ok(req.device_id.clone())
}

/// Builds the compound `channel_id` used as the oneshot lookup key.
///
/// Format: `"mobile:<device_id>:<request_id>"`
pub fn make_channel_id(device_id: &str, request_id: &str) -> ChannelId {
    ChannelId::from(format!("mobile:{device_id}:{request_id}"))
}

/// Builds the stable `session_id` for a device.
///
/// Format: `"mobile:<device_id>"`
pub fn make_session_id(device_id: &str) -> SessionId {
    SessionId::from(format!("mobile:{device_id}"))
}

/// Extracts `(device_id, request_id)` from a compound mobile `channel_id` string.
///
/// Returns `None` if the string is not in the expected `"mobile:<d>:<r>"` format.
pub fn parse_mobile_channel_id(channel_str: &str) -> Option<(&str, &str)> {
    let rest = channel_str.strip_prefix("mobile:")?;
    // Split at the LAST ':' so device_id can itself contain ':'
    let sep = rest.rfind(':')?;
    Some((&rest[..sep], &rest[sep + 1..]))
}

/// Reads one length-prefixed frame from a QUIC receive stream.
pub async fn read_frame(recv: &mut quinn::RecvStream) -> Result<Vec<u8>, ChannelError> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?;

    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        return Err(ChannelError::ConnectionFailed("Frame too large".into()));
    }

    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?;

    Ok(buf)
}

/// Writes one length-prefixed frame to a QUIC send stream.
pub async fn write_frame(send: &mut quinn::SendStream, payload: &[u8]) -> Result<(), ChannelError> {
    let len = (payload.len() as u32).to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
    send.write_all(payload)
        .await
        .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
    Ok(())
}

/// Deserializes a `ClientMessage` from raw JSON bytes.
fn parse_client_message(buf: &[u8]) -> Result<ClientMessage, ChannelError> {
    serde_json::from_slice(buf)
        .map_err(|e| ChannelError::ConnectionFailed(format!("Deserialize error: {e}")))
}

/// Serializes a `ServerMessage` to JSON bytes.
fn encode_server_message(msg: &ServerMessage) -> Result<Vec<u8>, ChannelError> {
    serde_json::to_vec(msg).map_err(|e| ChannelError::SendFailed(format!("Serialize error: {e}")))
}

// ─── TLS helpers ─────────────────────────────────────────────────────────────

fn generate_self_signed() -> Result<
    (
        rustls::pki_types::CertificateDer<'static>,
        rustls::pki_types::PrivateKeyDer<'static>,
    ),
    Box<dyn std::error::Error>,
> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.key_pair.serialize_der())?;
    Ok((cert_der, key_der))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{ChannelId as CId, SessionId as SId};

    // ── Auth validation ──────────────────────────────────────────────────────

    #[test]
    fn validate_auth_accepts_valid_token() {
        let req = AuthRequest {
            token: "secret".into(),
            device_id: "device1".into(),
        };
        let result = validate_auth(&req, "secret");
        assert_eq!(result.unwrap(), "device1");
    }

    #[test]
    fn validate_auth_rejects_wrong_token() {
        let req = AuthRequest {
            token: "wrong".into(),
            device_id: "device1".into(),
        };
        let err = validate_auth(&req, "secret").unwrap_err();
        assert!(err.to_string().contains("Invalid API token"));
    }

    #[test]
    fn validate_auth_rejects_empty_device_id() {
        let req = AuthRequest {
            token: "secret".into(),
            device_id: "".into(),
        };
        let err = validate_auth(&req, "secret").unwrap_err();
        assert!(err.to_string().contains("device_id"));
    }

    // ── ID construction ──────────────────────────────────────────────────────

    #[test]
    fn make_channel_id_formats_compound_id() {
        let id = make_channel_id("dev123", "req456");
        assert_eq!(id.as_str(), "mobile:dev123:req456");
    }

    #[test]
    fn make_session_id_uses_mobile_prefix() {
        let sid = make_session_id("dev123");
        assert_eq!(sid.as_str(), "mobile:dev123");
    }

    // ── channel_id parsing ───────────────────────────────────────────────────

    #[test]
    fn parse_mobile_channel_id_extracts_parts() {
        let (device_id, request_id) = parse_mobile_channel_id("mobile:dev123:req456").unwrap();
        assert_eq!(device_id, "dev123");
        assert_eq!(request_id, "req456");
    }

    #[test]
    fn parse_mobile_channel_id_handles_device_id_with_colon() {
        // device_id may contain colons; we split at the LAST ':'
        let (device_id, request_id) =
            parse_mobile_channel_id("mobile:org:device:req-uuid").unwrap();
        assert_eq!(device_id, "org:device");
        assert_eq!(request_id, "req-uuid");
    }

    #[test]
    fn parse_mobile_channel_id_rejects_wrong_prefix() {
        assert!(parse_mobile_channel_id("telegram:123").is_none());
    }

    #[test]
    fn parse_mobile_channel_id_rejects_missing_request_part() {
        // Only one segment after "mobile:" — no second ':'
        assert!(parse_mobile_channel_id("mobile:only_device").is_none());
    }

    // ── Wire protocol ────────────────────────────────────────────────────────

    #[test]
    fn parse_client_message_deserializes_auth() {
        let json = r#"{"type":"auth","token":"tok","device_id":"d1"}"#;
        let msg = parse_client_message(json.as_bytes()).unwrap();
        assert!(matches!(msg, ClientMessage::Auth(_)));
        if let ClientMessage::Auth(req) = msg {
            assert_eq!(req.token, "tok");
            assert_eq!(req.device_id, "d1");
        }
    }

    #[test]
    fn parse_client_message_deserializes_message() {
        let json = r#"{"type":"message","text":"hello"}"#;
        let msg = parse_client_message(json.as_bytes()).unwrap();
        assert!(matches!(msg, ClientMessage::Message(_)));
        if let ClientMessage::Message(m) = msg {
            assert_eq!(m.text, "hello");
        }
    }

    #[test]
    fn parse_client_message_rejects_invalid_json() {
        let err = parse_client_message(b"{bad").unwrap_err();
        assert!(err.to_string().contains("Deserialize error"));
    }

    #[test]
    fn encode_server_message_serializes_response() {
        let msg = ServerMessage::Response(ResponsePayload {
            content: "pong".into(),
            is_error: false,
        });
        let bytes = encode_server_message(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["type"], "response");
        assert_eq!(parsed["content"], "pong");
        assert_eq!(parsed["is_error"], false);
    }

    #[test]
    fn encode_server_message_serializes_auth_ok() {
        let msg = ServerMessage::AuthOk(AuthOkPayload {
            session_id: "mobile:d1".into(),
        });
        let bytes = encode_server_message(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["type"], "auth_ok");
        assert_eq!(parsed["session_id"], "mobile:d1");
    }

    // ── Oneshot routing ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn send_response_resolves_pending_oneshot() {
        let adapter = MobileAdapter::new("127.0.0.1:0".parse().unwrap(), "tok".into());
        let key = "mobile:d1:req1".to_string();
        let (tx, rx) = oneshot::channel::<AgentResponse>();
        adapter.inner.pending.insert(key.clone(), tx);

        let resp = AgentResponse::new(CId::from(key.as_str()), SId::from("mobile:d1"), "hello");
        adapter.send_response(resp.clone()).await.unwrap();

        let received = rx.await.unwrap();
        assert_eq!(received.content, "hello");
        assert!(!received.is_error);
    }

    #[tokio::test]
    async fn send_response_ignores_missing_key() {
        let adapter = MobileAdapter::new("127.0.0.1:0".parse().unwrap(), "tok".into());
        let resp = AgentResponse::new(
            CId::from("mobile:d1:unknown"),
            SId::from("mobile:d1"),
            "noop",
        );
        // Should not panic or error — just warn
        adapter.send_response(resp).await.unwrap();
    }
}
