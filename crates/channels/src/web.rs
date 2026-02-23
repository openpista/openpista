//! Web channel adapter with WebSocket and static file serving.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use serde::Deserialize;
use tokio::sync::mpsc;
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::adapter::ChannelAdapter;

type ClientSender = mpsc::UnboundedSender<Message>;

/// Configuration for the Web adapter.
///
/// Mirrors `WebConfig` from `crates/cli/src/config.rs`.
#[derive(Debug, Clone)]
pub struct WebAdapterConfig {
    /// HTTP/WebSocket server port.
    pub port: u16,
    /// Bearer token for WebSocket handshake authentication.
    pub token: String,
    /// Allowed CORS origins (comma-separated). Empty = allow all.
    pub cors_origins: String,
    /// Directory for serving WASM bundle and H5 static assets.
    pub static_dir: String,
}

/// Web channel adapter — axum-based HTTP/WebSocket server.
///
/// Serves a WebSocket endpoint at `/ws` and static files from `static_dir`.
/// Connected clients are tracked via a shared `DashMap` keyed by client UUID
/// so that `send_response` can route to the correct WebSocket.
pub struct WebAdapter {
    port: u16,
    token: String,
    cors_origins: String,
    static_dir: String,
    clients: Arc<DashMap<String, ClientSender>>,
    #[allow(dead_code)]
    resp_tx: mpsc::Sender<AgentResponse>,
}

impl WebAdapter {
    /// Creates a new Web adapter from config and response channel.
    pub fn new(config: WebAdapterConfig, resp_tx: mpsc::Sender<AgentResponse>) -> Self {
        Self {
            port: config.port,
            token: config.token,
            cors_origins: config.cors_origins,
            static_dir: config.static_dir,
            clients: Arc::new(DashMap::new()),
            resp_tx,
        }
    }

    /// Creates a second adapter instance that shares the same client map.
    /// Use this for the response-routing copy kept in the forwarder task.
    pub fn clone_for_responses(&self) -> Self {
        Self {
            port: self.port,
            token: self.token.clone(),
            cors_origins: self.cors_origins.clone(),
            static_dir: self.static_dir.clone(),
            clients: Arc::clone(&self.clients),
            resp_tx: self.resp_tx.clone(),
        }
    }

    /// Creates a stable session id for a web client UUID.
    fn make_session_id(client_id: &str) -> SessionId {
        SessionId::from(format!("web:{client_id}"))
    }
}

// ─── Axum shared state ─────────────────────────────────────

#[derive(Clone)]
struct WebState {
    token: String,
    event_tx: mpsc::Sender<ChannelEvent>,
    clients: Arc<DashMap<String, ClientSender>>,
}

#[derive(Deserialize)]
struct WsQuery {
    token: Option<String>,
}

// ─── ChannelAdapter impl ───────────────────────────────────

#[async_trait]
impl ChannelAdapter for WebAdapter {
    fn channel_id(&self) -> ChannelId {
        ChannelId::new("web", "ws")
    }

    async fn run(self, tx: mpsc::Sender<ChannelEvent>) -> Result<(), ChannelError> {
        info!("Web adapter starting on port {}", self.port);

        let state = WebState {
            token: self.token.clone(),
            event_tx: tx,
            clients: Arc::clone(&self.clients),
        };

        let cors = build_cors_layer(&self.cors_origins);
        let serve_dir = ServeDir::new(&self.static_dir);

        let app = Router::new()
            .route("/ws", get(ws_upgrade))
            .with_state(state)
            .fallback_service(serve_dir)
            .layer(cors);

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], self.port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("bind failed: {e}")))?;

        info!("Web adapter listening on {addr}");
        axum::serve(listener, app)
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("server error: {e}")))?;

        info!("Web adapter stopped");
        Ok(())
    }

    async fn send_response(&self, resp: AgentResponse) -> Result<(), ChannelError> {
        let client_id = parse_client_id(resp.channel_id.as_str())?;

        let json = serde_json::to_string(&resp)
            .map_err(|e| ChannelError::SendFailed(format!("JSON serialization error: {e}")))?;

        if let Some(sender) = self.clients.get(&client_id) {
            sender
                .send(Message::Text(json.into()))
                .map_err(|e| ChannelError::SendFailed(format!("WebSocket send error: {e}")))?;
            Ok(())
        } else {
            Err(ChannelError::SendFailed(format!(
                "Client {client_id} not connected"
            )))
        }
    }
}

// ─── Axum handlers ─────────────────────────────────────────

/// GET /ws — WebSocket upgrade with token authentication.
async fn ws_upgrade(
    State(state): State<WebState>,
    Query(params): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    // Token auth: check query param or Sec-WebSocket-Protocol header
    if !state.token.is_empty() {
        let token_from_query = params.token.as_deref().unwrap_or("");
        let token_from_header = headers
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if token_from_query != state.token && token_from_header != state.token {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    let client_id = Uuid::new_v4().to_string();
    debug!(client_id = %client_id, "WebSocket connection accepted");

    ws.on_upgrade(move |socket| handle_ws(socket, client_id, state))
        .into_response()
}

/// Manages a single WebSocket connection: read loop, write relay, heartbeat.
async fn handle_ws(socket: WebSocket, client_id: String, state: WebState) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<Message>();

    // Register client
    state.clients.insert(client_id.clone(), msg_tx);
    info!(client_id = %client_id, "WebSocket client connected");

    // Spawn writer relay: forwards queued messages to the WebSocket sink
    let write_client_id = client_id.clone();
    let write_handle = tokio::spawn(async move {
        while let Some(msg) = msg_rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                debug!(client_id = %write_client_id, "WebSocket write failed, closing");
                break;
            }
        }
    });

    // Heartbeat: ping every 30 s
    let heartbeat_clients = Arc::clone(&state.clients);
    let heartbeat_id = client_id.clone();
    let ping_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Some(sender) = heartbeat_clients.get(&heartbeat_id) {
                if sender.send(Message::Ping(Vec::new().into())).is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    });

    // Read loop
    let read_client_id = client_id.clone();
    while let Some(result) = ws_rx.next().await {
        match result {
            Ok(Message::Text(text)) => {
                let channel_id = ChannelId::new("web", &read_client_id);
                let session_id = WebAdapter::make_session_id(&read_client_id);
                let user_text = parse_ws_message(&text);
                let event = ChannelEvent::new(channel_id, session_id, user_text);
                if let Err(e) = state.event_tx.send(event).await {
                    error!("Failed to forward web event: {e}");
                }
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Close(_)) => break,
            Err(e) => {
                debug!(client_id = %read_client_id, error = %e, "WebSocket read error");
                break;
            }
            _ => {}
        }
    }

    // Cleanup
    state.clients.remove(&client_id);
    ping_handle.abort();
    write_handle.abort();
    info!(client_id = %client_id, "WebSocket client disconnected");
}

// ─── Helpers ───────────────────────────────────────────────

/// Parses a WebSocket text message.
/// Expects JSON `{"user_message": "..."}`, falls back to raw text.
fn parse_ws_message(text: &str) -> String {
    #[derive(Deserialize)]
    struct WsMsg {
        user_message: Option<String>,
    }
    serde_json::from_str::<WsMsg>(text)
        .ok()
        .and_then(|m| m.user_message)
        .unwrap_or_else(|| text.to_string())
}

/// Parses a client UUID from `web:<id>` or raw string.
fn parse_client_id(channel_str: &str) -> Result<String, ChannelError> {
    let client_id = channel_str.strip_prefix("web:").unwrap_or(channel_str);
    if client_id.is_empty() {
        Err(ChannelError::SendFailed(
            "Empty client id in channel id".to_string(),
        ))
    } else {
        Ok(client_id.to_string())
    }
}

/// Builds a CORS layer from comma-separated origins. Empty = allow all.
fn build_cors_layer(origins: &str) -> CorsLayer {
    use tower_http::cors::Any;
    if origins.is_empty() {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        let parsed: Vec<_> = origins
            .split(',')
            .filter_map(|o| o.trim().parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(parsed)
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_session_id_uses_web_prefix() {
        let sid = WebAdapter::make_session_id("abc-123");
        assert_eq!(sid.as_str(), "web:abc-123");
    }

    #[test]
    fn parse_client_id_handles_prefixed_and_raw() {
        assert_eq!(parse_client_id("web:abc-123").unwrap(), "abc-123");
        assert_eq!(parse_client_id("abc-123").unwrap(), "abc-123");
    }

    #[test]
    fn parse_client_id_rejects_empty() {
        assert!(parse_client_id("web:").is_err());
    }

    #[test]
    fn parse_ws_message_extracts_json_user_message() {
        let msg = r#"{"user_message": "hello world"}"#;
        assert_eq!(parse_ws_message(msg), "hello world");
    }

    #[test]
    fn parse_ws_message_falls_back_to_raw_text() {
        assert_eq!(parse_ws_message("raw text"), "raw text");
    }

    #[test]
    fn parse_ws_message_handles_json_without_user_message() {
        assert_eq!(
            parse_ws_message(r#"{"other": "field"}"#),
            r#"{"other": "field"}"#
        );
    }

    #[test]
    fn build_cors_layer_does_not_panic() {
        let _ = build_cors_layer("");
        let _ = build_cors_layer("http://localhost:3000");
        let _ = build_cors_layer("http://localhost:3000,http://example.com");
    }
}
