//! Web channel adapter — axum HTTP server with WebSocket support.
//!
//! Provides a WebSocket-based channel for browser clients, served alongside
//! static H5 chat assets from a configurable directory.

use async_trait::async_trait;
use axum::{
    Router,
    extract::{Query, State, WebSocketUpgrade, ws},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::adapter::ChannelAdapter;

// ─── WsMessage envelope ────────────────────────────────────

/// WebSocket message envelope for client-server communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Client sends a chat message.
    #[serde(rename = "message")]
    UserMessage { content: String },
    /// Server sends an agent response.
    #[serde(rename = "response")]
    AgentReply { content: String, is_error: bool },
    /// Heartbeat ping from client.
    #[serde(rename = "ping")]
    Ping,
    /// Heartbeat pong from server.
    #[serde(rename = "pong")]
    Pong,
    /// Authentication request from client.
    #[serde(rename = "auth")]
    Auth { token: String },
    /// Authentication result from server.
    #[serde(rename = "auth_result")]
    AuthResult {
        success: bool,
        client_id: Option<String>,
        error: Option<String>,
    },
}

// ─── Query parameters ──────────────────────────────────────

/// Query parameters for WebSocket upgrade request.
#[derive(Debug, Deserialize)]
pub struct WsConnectParams {
    /// Authentication token (passed as `?token=xxx`).
    pub token: Option<String>,
    /// Optional client ID for session persistence.
    pub client_id: Option<String>,
}

// ─── Shared state ──────────────────────────────────────────

/// Shared state for the axum web server.
struct WebState {
    /// Expected authentication token.
    auth_token: String,
    /// Event sender to the core engine.
    event_tx: mpsc::Sender<ChannelEvent>,
    /// Connected clients: client_id -> per-client sender.
    clients: Arc<DashMap<String, mpsc::Sender<AgentResponse>>>,
}

// ─── WebAdapter ────────────────────────────────────────────

/// Web channel adapter — runs an axum HTTP server with WebSocket support.
#[derive(Clone)]
pub struct WebAdapter {
    port: u16,
    auth_token: String,
    cors_origins: String,
    static_dir: String,
    /// Broadcast sender for responses from the agent (used by `send_response`).
    response_tx: broadcast::Sender<AgentResponse>,
    /// Connected clients — shared with the axum handlers.
    pub clients: Arc<DashMap<String, mpsc::Sender<AgentResponse>>>,
}

impl WebAdapter {
    /// Creates a new web adapter with the given parameters.
    pub fn new(
        port: u16,
        auth_token: String,
        cors_origins: String,
        static_dir: String,
    ) -> Self {
        let (response_tx, _) = broadcast::channel(256);
        Self {
            port,
            auth_token,
            cors_origins,
            static_dir,
            response_tx,
            clients: Arc::new(DashMap::new()),
        }
    }

    /// Creates a stable session id for a web client.
    fn make_session_id(client_id: &str) -> SessionId {
        SessionId::from(format!("web:{client_id}"))
    }

    /// Builds the CORS layer from the configured origins string.
    fn build_cors(&self) -> CorsLayer {
        if self.cors_origins == "*" {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        } else {
            // Parse comma-separated origins
            let origins: Vec<_> = self
                .cors_origins
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(Any)
                .allow_headers(Any)
        }
    }
}

#[async_trait]
impl ChannelAdapter for WebAdapter {
    fn channel_id(&self) -> ChannelId {
        ChannelId::new("web", "server")
    }

    async fn run(self, tx: mpsc::Sender<ChannelEvent>) -> Result<(), ChannelError> {
        info!(port = self.port, "Web adapter starting");

        let cors = self.build_cors();
        let state = Arc::new(WebState {
            auth_token: self.auth_token.clone(),
            event_tx: tx,
            clients: self.clients.clone(),
        });

        let app = Router::new()
            .route("/ws", get(ws_handler))
            .route("/health", get(health_handler))
            .with_state(state)
            .layer(cors);

        // Optionally serve static files from configured directory
        let app = if !self.static_dir.is_empty() {
            let expanded = expand_tilde(&self.static_dir);
            let static_path = std::path::Path::new(&expanded);
            if static_path.exists() {
                info!(path = %expanded, "Serving static files");
                app.fallback_service(tower_http::services::ServeDir::new(expanded))
            } else {
                debug!(path = %expanded, "Static dir does not exist, skipping");
                app
            }
        } else {
            app
        };

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.port))
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("bind failed: {e}")))?;

        info!(port = self.port, "Web adapter listening");

        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("server error: {e}")))?;

        info!("Web adapter stopped");
        Ok(())
    }

    async fn send_response(&self, resp: AgentResponse) -> Result<(), ChannelError> {
        // Route response to the correct per-client sender
        let client_id = resp
            .channel_id
            .as_str()
            .strip_prefix("web:")
            .unwrap_or("")
            .to_string();

        if let Some(sender) = self.clients.get(&client_id) {
            sender
                .send(resp)
                .await
                .map_err(|e| ChannelError::SendFailed(format!("client send failed: {e}")))?;
        } else {
            // Broadcast fallback
            let _ = self.response_tx.send(resp);
        }
        Ok(())
    }
}

// ─── Axum handlers ─────────────────────────────────────────

/// Health check endpoint.
async fn health_handler() -> &'static str {
    "ok"
}

/// WebSocket upgrade handler.
async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsConnectParams>,
    State(state): State<Arc<WebState>>,
) -> impl IntoResponse {
    // Validate token from query parameter
    let token_valid = params
        .token
        .as_deref()
        .is_some_and(|t| validate_token(t, &state.auth_token));

    if !token_valid && !state.auth_token.is_empty() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let client_id = params
        .client_id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    ws.on_upgrade(move |socket| handle_ws(socket, client_id, state))
}

/// Manages a single WebSocket connection lifecycle.
async fn handle_ws(socket: ws::WebSocket, client_id: String, state: Arc<WebState>) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Per-client response channel
    let (resp_tx, mut resp_rx) = mpsc::channel::<AgentResponse>(64);
    state.clients.insert(client_id.clone(), resp_tx);

    info!(client_id = %client_id, "WebSocket client connected");

    // Send auth_result to client
    let auth_msg = WsMessage::AuthResult {
        success: true,
        client_id: Some(client_id.clone()),
        error: None,
    };
    if let Ok(json) = serde_json::to_string(&auth_msg) {
        let _ = ws_tx.send(ws::Message::Text(json.into())).await;
    }

    let client_id_read = client_id.clone();
    let state_read = state.clone();

    // Read task: client -> server
    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            let ws::Message::Text(text) = msg else {
                continue;
            };
            let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) else {
                warn!(client_id = %client_id_read, "Invalid WS message");
                continue;
            };
            match ws_msg {
                WsMessage::UserMessage { content } => {
                    let channel_id = ChannelId::new("web", &client_id_read);
                    let session_id = WebAdapter::make_session_id(&client_id_read);
                    let event = ChannelEvent::new(channel_id, session_id, content);
                    if let Err(e) = state_read.event_tx.send(event).await {
                        error!("Failed to send web event: {e}");
                        break;
                    }
                }
                WsMessage::Ping => {
                    debug!(client_id = %client_id_read, "Ping received");
                }
                _ => {
                    debug!(client_id = %client_id_read, "Ignoring WS message");
                }
            }
        }
    });

    // Write task: server -> client
    let write_task = tokio::spawn(async move {
        while let Some(resp) = resp_rx.recv().await {
            let ws_msg = WsMessage::AgentReply {
                content: resp.content,
                is_error: resp.is_error,
            };
            let Ok(json) = serde_json::to_string(&ws_msg) else {
                continue;
            };
            if ws_tx.send(ws::Message::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = read_task => {},
        _ = write_task => {},
    }

    state.clients.remove(&client_id);
    info!(client_id = %client_id, "WebSocket client disconnected");
}

// ─── Helpers ───────────────────────────────────────────────

/// Token comparison for authentication.
fn validate_token(given: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return true; // no auth required
    }
    given == expected
}

/// Expands `~` at the start of a path to `$HOME`.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}{rest}")
    } else {
        path.to_string()
    }
}

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_message_user_message_serializes_correctly() {
        let msg = WsMessage::UserMessage {
            content: "hello".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("\"content\":\"hello\""));
    }

    #[test]
    fn ws_message_agent_reply_serializes_correctly() {
        let msg = WsMessage::AgentReply {
            content: "world".to_string(),
            is_error: false,
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("\"type\":\"response\""));
        assert!(json.contains("\"is_error\":false"));
    }

    #[test]
    fn ws_message_ping_pong_roundtrip() {
        let ping = serde_json::to_string(&WsMessage::Ping).expect("serialize ping");
        let parsed: WsMessage = serde_json::from_str(&ping).expect("deserialize ping");
        assert!(matches!(parsed, WsMessage::Ping));

        let pong = serde_json::to_string(&WsMessage::Pong).expect("serialize pong");
        let parsed: WsMessage = serde_json::from_str(&pong).expect("deserialize pong");
        assert!(matches!(parsed, WsMessage::Pong));
    }

    #[test]
    fn ws_message_auth_result_roundtrip() {
        let msg = WsMessage::AuthResult {
            success: true,
            client_id: Some("abc".to_string()),
            error: None,
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(
            parsed,
            WsMessage::AuthResult { success: true, .. }
        ));
    }

    #[test]
    fn validate_token_allows_empty_expected() {
        assert!(validate_token("anything", ""));
        assert!(validate_token("", ""));
    }

    #[test]
    fn validate_token_checks_exact_match() {
        assert!(validate_token("secret", "secret"));
        assert!(!validate_token("wrong", "secret"));
        assert!(!validate_token("", "secret"));
    }

    #[test]
    fn expand_tilde_replaces_home() {
        let expanded = expand_tilde("~/.openpista/web");
        assert!(!expanded.starts_with('~'));
        assert!(expanded.ends_with("/.openpista/web"));
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths_unchanged() {
        assert_eq!(expand_tilde("/var/www"), "/var/www");
    }

    #[test]
    fn make_session_id_uses_web_prefix() {
        let sid = WebAdapter::make_session_id("abc123");
        assert_eq!(sid.as_str(), "web:abc123");
    }

    #[test]
    fn channel_id_uses_web_prefix() {
        let channel = ChannelId::new("web", "test-client");
        assert_eq!(channel.as_str(), "web:test-client");
    }

    #[test]
    fn web_adapter_creates_with_defaults() {
        let adapter = WebAdapter::new(
            3210,
            "token123".to_string(),
            "*".to_string(),
            "~/.openpista/web".to_string(),
        );
        assert_eq!(adapter.port, 3210);
        assert_eq!(adapter.auth_token, "token123");
        assert!(adapter.clients.is_empty());
    }
}
