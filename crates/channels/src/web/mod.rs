//! Web channel adapter — axum HTTP server with WebSocket support.
//!
//! Provides a WebSocket-based channel for browser clients, served alongside
//! static H5 chat assets from a configurable directory.

pub mod handlers;
pub mod types;
pub mod websocket;

pub use types::{
    ModelChangeCallback, ProviderAuthCallback, ProviderAuthIntent, ProviderAuthResult,
    ProviderListCallback, SessionLoader, WebApprovalHandler, WebHistoryMessage, WebModelEntry,
    WebProviderAuthEntry, WebSessionEntry, WsConnectParams, WsMessage,
};

use async_trait::async_trait;
use axum::{Router, routing::get};
use dashmap::DashMap;
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

use crate::adapter::ChannelAdapter;
use types::{AuthSession, PendingApprovals, WebState};

// ─── WebAdapter ────────────────────────────────────────────

/// Web channel adapter — runs an axum HTTP server with WebSocket support.
#[derive(Clone)]
pub struct WebAdapter {
    port: u16,
    auth_token: String,
    cors_origins: String,
    static_dir: String,
    shared_session_id: String,
    sessions: Arc<RwLock<Arc<Vec<WebSessionEntry>>>>,
    selected_provider: Option<String>,
    selected_model: Option<String>,
    response_tx: broadcast::Sender<AgentResponse>,
    pub clients: Arc<DashMap<String, mpsc::Sender<AgentResponse>>>,
    session_loader: Option<Arc<dyn SessionLoader>>,
    model_change_cb: Option<ModelChangeCallback>,
    model_list: Arc<Vec<WebModelEntry>>,
    provider_list_cb: Option<ProviderListCallback>,
    provider_auth_cb: Option<ProviderAuthCallback>,
    approval_broadcast: broadcast::Sender<WsMessage>,
    pending_approvals: PendingApprovals,
}
impl WebAdapter {
    pub fn new(
        port: u16,
        auth_token: String,
        cors_origins: String,
        static_dir: String,
        shared_session_id: String,
    ) -> Self {
        let (response_tx, _) = broadcast::channel(256);
        let (approval_broadcast, _) = broadcast::channel(64);
        Self {
            port,
            auth_token,
            cors_origins,
            static_dir,
            shared_session_id,
            sessions: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            selected_provider: None,
            selected_model: None,
            response_tx,
            clients: Arc::new(DashMap::new()),
            session_loader: None,
            model_change_cb: None,
            model_list: Arc::new(Vec::new()),
            provider_list_cb: None,
            provider_auth_cb: None,
            approval_broadcast,
            pending_approvals: Arc::new(DashMap::new()),
        }
    }

    pub fn with_session_loader(mut self, loader: Arc<dyn SessionLoader>) -> Self {
        self.session_loader = Some(loader);
        self
    }

    pub fn with_model_change_callback(mut self, cb: ModelChangeCallback) -> Self {
        self.model_change_cb = Some(cb);
        self
    }

    pub fn with_model_list(mut self, models: Vec<WebModelEntry>) -> Self {
        self.model_list = Arc::new(models);
        self
    }

    pub fn with_provider_list_callback(mut self, cb: ProviderListCallback) -> Self {
        self.provider_list_cb = Some(cb);
        self
    }

    pub fn with_provider_auth_callback(mut self, cb: ProviderAuthCallback) -> Self {
        self.provider_auth_cb = Some(cb);
        self
    }

    pub fn with_selected_model(mut self, provider: String, model: String) -> Self {
        self.selected_provider = if provider.trim().is_empty() {
            None
        } else {
            Some(provider)
        };
        self.selected_model = if model.trim().is_empty() {
            None
        } else {
            Some(model)
        };
        self
    }

    pub fn set_sessions(&self, sessions: Vec<WebSessionEntry>) {
        let arc = Arc::new(sessions);
        match self.sessions.write() {
            Ok(mut guard) => *guard = arc,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = arc;
            }
        }
    }

    fn make_session_id(client_id: &str) -> SessionId {
        SessionId::from(format!("web:{client_id}"))
    }

    pub(crate) fn resolve_session_id(
        query_session_id: Option<&str>,
        shared_session_id: &str,
        client_id: &str,
    ) -> SessionId {
        if let Some(query) = normalize_non_empty(query_session_id) {
            return SessionId::from(query);
        }
        if let Some(shared) = normalize_non_empty(Some(shared_session_id)) {
            return SessionId::from(shared);
        }
        Self::make_session_id(client_id)
    }

    pub(crate) fn is_shared_with_tui(session_id: &SessionId, shared_session_id: &str) -> bool {
        normalize_non_empty(Some(shared_session_id))
            .is_some_and(|shared| session_id.as_str() == shared)
    }

    pub fn approval_handler(&self) -> Arc<WebApprovalHandler> {
        Arc::new(WebApprovalHandler::new(
            self.approval_broadcast.clone(),
            self.pending_approvals.clone(),
        ))
    }

    fn build_cors(&self) -> CorsLayer {
        if self.cors_origins == "*" {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        } else {
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

        let expanded_static_dir = proto::path::expand_tilde(&self.static_dir);

        let cached_index_html = if !expanded_static_dir.is_empty() {
            let path = std::path::PathBuf::from(&expanded_static_dir).join("index.html");
            tokio::fs::read_to_string(&path).await.ok()
        } else {
            None
        };

        let cors = self.build_cors();
        let auth_sessions: Arc<DashMap<String, AuthSession>> = Arc::new(DashMap::new());
        let state = Arc::new(WebState {
            auth_token: self.auth_token.clone(),
            event_tx: tx,
            auth_sessions: auth_sessions.clone(),
            clients: self.clients.clone(),
            selected_provider: Arc::new(RwLock::new(self.selected_provider.clone())),
            selected_model: Arc::new(RwLock::new(self.selected_model.clone())),
            shared_session_id: self.shared_session_id.clone(),
            sessions: self.sessions.clone(),
            session_loader: self.session_loader.clone(),
            static_dir: expanded_static_dir.clone(),
            index_html: cached_index_html,
            model_change_cb: self.model_change_cb.clone(),
            model_list: self.model_list.clone(),
            provider_list_cb: self.provider_list_cb.clone(),
            provider_auth_cb: self.provider_auth_cb.clone(),
            pending_approvals: self.pending_approvals.clone(),
            approval_broadcast: self.approval_broadcast.clone(),
        });

        let app = Router::new()
            .route("/ws", get(websocket::ws_handler))
            .route("/health", get(handlers::health_handler))
            .route(
                "/auth",
                get(handlers::auth_page_handler).post(handlers::auth_handler),
            )
            .route("/s/{session_id}", get(handlers::session_page_handler))
            .with_state(state)
            .layer(cors);

        let app = if !expanded_static_dir.is_empty() {
            info!(path = %expanded_static_dir, "Serving static files");
            app.fallback_service(tower_http::services::ServeDir::new(expanded_static_dir))
        } else {
            app
        };

        let cleanup_sessions = auth_sessions.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                let now = chrono::Utc::now();
                let before = cleanup_sessions.len();
                cleanup_sessions.retain(|_, session| session.expires_at > now);
                let removed = before - cleanup_sessions.len();
                if removed > 0 {
                    debug!(removed, "Cleaned up expired auth sessions");
                }
            }
        });

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
        let client_id = resp
            .channel_id
            .as_str()
            .strip_prefix("web:")
            .unwrap_or("")
            .to_string();
        let channel_id = resp.channel_id.clone();
        let session_id = resp.session_id.clone();

        if let Some(sender) = self.clients.get(&client_id).map(|r| r.clone()) {
            if let Err(e) = sender.send(resp).await {
                error!(
                    client_id = %client_id,
                    channel_id = %channel_id,
                    session_id = %session_id,
                    "Failed to route response to web client queue: {e}"
                );
                return Err(ChannelError::SendFailed(format!("client send failed: {e}")));
            }
        } else {
            warn!(
                client_id = %client_id,
                channel_id = %channel_id,
                session_id = %session_id,
                "Web client queue missing; using broadcast fallback"
            );
            let _ = self.response_tx.send(resp);
        }
        Ok(())
    }
}

fn normalize_non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::handlers::validate_token;
    use super::types::to_web_history;
    use super::*;
    use tokio::time::{Duration, timeout};

    #[test]
    fn ws_message_user_message_serializes_correctly() {
        let msg = WsMessage::UserMessage {
            content: "hello".to_string(),
            message_id: None,
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
            provider: Some("anthropic".to_string()),
            model: Some("claude-sonnet-4-6".to_string()),
            session_id: Some("shared-main".to_string()),
            shared_with_tui: Some(true),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            WsMessage::AuthResult {
                success,
                provider,
                model,
                session_id,
                shared_with_tui,
                ..
            } => {
                assert!(success);
                assert_eq!(provider.as_deref(), Some("anthropic"));
                assert_eq!(model.as_deref(), Some("claude-sonnet-4-6"));
                assert_eq!(session_id.as_deref(), Some("shared-main"));
                assert_eq!(shared_with_tui, Some(true));
            }
            _ => panic!("expected AuthResult"),
        }
    }

    #[test]
    fn ws_message_sessions_request_roundtrip() {
        let json = serde_json::to_string(&WsMessage::SessionsRequest).expect("serialize");
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(parsed, WsMessage::SessionsRequest));
    }

    #[test]
    fn ws_message_sessions_list_roundtrip() {
        let msg = WsMessage::SessionsList {
            sessions: vec![WebSessionEntry {
                id: "session-a".to_string(),
                channel_id: "cli:tui".to_string(),
                updated_at: "2026-02-25T00:00:00Z".to_string(),
                preview: "hello".to_string(),
            }],
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            WsMessage::SessionsList { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].id, "session-a");
                assert_eq!(sessions[0].channel_id, "cli:tui");
            }
            _ => panic!("expected SessionsList"),
        }
    }

    #[test]
    fn ws_message_ack_roundtrip() {
        let msg = WsMessage::MessageAck {
            message_id: Some("m1".to_string()),
            session_id: "shared-main".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            WsMessage::MessageAck {
                message_id,
                session_id,
            } => {
                assert_eq!(message_id.as_deref(), Some("m1"));
                assert_eq!(session_id, "shared-main");
            }
            _ => panic!("expected MessageAck"),
        }
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
        let expanded = proto::path::expand_tilde("~/.openpista/web");
        assert!(!expanded.starts_with('~'));
        assert!(expanded.ends_with("/.openpista/web"));
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths_unchanged() {
        assert_eq!(proto::path::expand_tilde("/var/www"), "/var/www");
    }

    #[test]
    fn make_session_id_uses_web_prefix() {
        let sid = WebAdapter::make_session_id("abc123");
        assert_eq!(sid.as_str(), "web:abc123");
    }

    #[test]
    fn resolve_session_id_prefers_query_over_shared_session() {
        let sid = WebAdapter::resolve_session_id(Some("query-session"), "shared-main", "client-a");
        assert_eq!(sid.as_str(), "query-session");
    }

    #[test]
    fn resolve_session_id_uses_shared_session_when_query_missing() {
        let sid = WebAdapter::resolve_session_id(None, "shared-main", "client-a");
        assert_eq!(sid.as_str(), "shared-main");
    }

    #[test]
    fn resolve_session_id_uses_query_when_shared_empty() {
        let sid = WebAdapter::resolve_session_id(Some("query-session"), "   ", "client-a");
        assert_eq!(sid.as_str(), "query-session");
    }

    #[test]
    fn resolve_session_id_falls_back_to_client_prefix_when_shared_and_query_empty() {
        let sid = WebAdapter::resolve_session_id(None, "   ", "client-a");
        assert_eq!(sid.as_str(), "web:client-a");
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
            "shared-main".to_string(),
        );
        assert_eq!(adapter.port, 3210);
        assert_eq!(adapter.auth_token, "token123");
        assert_eq!(adapter.shared_session_id, "shared-main");
        assert!(adapter.selected_provider.is_none());
        assert!(adapter.selected_model.is_none());
        assert!(adapter.clients.is_empty());
        assert!(adapter.sessions.read().expect("sessions lock").is_empty());
    }

    #[test]
    fn with_selected_model_sets_metadata() {
        let adapter = WebAdapter::new(
            3210,
            "token".to_string(),
            "*".to_string(),
            "".to_string(),
            "shared-main".to_string(),
        )
        .with_selected_model("openai".to_string(), "gpt-4o".to_string());
        assert_eq!(adapter.selected_provider.as_deref(), Some("openai"));
        assert_eq!(adapter.selected_model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn set_sessions_replaces_snapshot() {
        let adapter = WebAdapter::new(
            3210,
            "token".to_string(),
            "*".to_string(),
            "".to_string(),
            "shared-main".to_string(),
        );
        adapter.set_sessions(vec![WebSessionEntry {
            id: "session-a".to_string(),
            channel_id: "cli:tui".to_string(),
            updated_at: "2026-02-25T00:00:00Z".to_string(),
            preview: "first".to_string(),
        }]);
        adapter.set_sessions(vec![WebSessionEntry {
            id: "session-b".to_string(),
            channel_id: "web:client-1".to_string(),
            updated_at: "2026-02-26T00:00:00Z".to_string(),
            preview: "second".to_string(),
        }]);

        let snapshot = adapter.sessions.read().expect("sessions lock");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, "session-b");
        assert_eq!(snapshot[0].preview, "second");
    }

    #[tokio::test]
    async fn send_response_routes_to_registered_client_queue() {
        let adapter = WebAdapter::new(
            3211,
            "token".to_string(),
            "*".to_string(),
            "".to_string(),
            "shared-main".to_string(),
        );
        let (tx, mut rx) = mpsc::channel::<AgentResponse>(2);
        adapter.clients.insert("client-a".to_string(), tx);

        let resp = AgentResponse::new(
            ChannelId::new("web", "client-a"),
            SessionId::from("web:client-a".to_string()),
            "hello web",
        );
        adapter
            .send_response(resp.clone())
            .await
            .expect("send response");

        let delivered = timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("timeout waiting for client queue")
            .expect("missing client response");
        assert_eq!(delivered.content, "hello web");
        assert!(!delivered.is_error);
    }

    #[tokio::test]
    async fn send_response_returns_error_when_client_queue_is_closed() {
        let adapter = WebAdapter::new(
            3212,
            "token".to_string(),
            "*".to_string(),
            "".to_string(),
            "shared-main".to_string(),
        );
        let (tx, rx) = mpsc::channel::<AgentResponse>(1);
        drop(rx);
        adapter.clients.insert("closed-client".to_string(), tx);

        let resp = AgentResponse::new(
            ChannelId::new("web", "closed-client"),
            SessionId::from("web:closed-client".to_string()),
            "should fail",
        );
        let err = adapter
            .send_response(resp)
            .await
            .expect_err("expected send error");
        assert!(matches!(err, ChannelError::SendFailed(_)));
    }

    #[tokio::test]
    async fn send_response_falls_back_to_broadcast_for_unknown_client() {
        let adapter = WebAdapter::new(
            3213,
            "token".to_string(),
            "*".to_string(),
            "".to_string(),
            "shared-main".to_string(),
        );
        let mut broadcast_rx = adapter.response_tx.subscribe();
        let resp = AgentResponse::new(
            ChannelId::new("web", "missing-client"),
            SessionId::from("web:missing-client".to_string()),
            "fallback broadcast",
        );

        adapter
            .send_response(resp.clone())
            .await
            .expect("send response");
        let delivered = timeout(Duration::from_millis(250), broadcast_rx.recv())
            .await
            .expect("timeout waiting for broadcast")
            .expect("broadcast channel closed");
        assert_eq!(delivered.content, "fallback broadcast");
    }

    #[test]
    fn web_history_message_serializes_correctly() {
        let msg = WebHistoryMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            tool_name: None,
            tool_call_id: None,
            created_at: "2026-02-27T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""role":"user""#));
        assert!(json.contains(r#""content":"hello""#));
        assert!(!json.contains("tool_name"));
        assert!(!json.contains("tool_call_id"));
    }

    #[test]
    fn ws_message_session_history_roundtrip() {
        let msg = WsMessage::SessionHistory {
            session_id: "session-a".to_string(),
            messages: vec![
                WebHistoryMessage {
                    role: "user".to_string(),
                    content: "hi".to_string(),
                    tool_name: None,
                    tool_call_id: None,
                    created_at: "2026-02-27T00:00:00Z".to_string(),
                },
                WebHistoryMessage {
                    role: "assistant".to_string(),
                    content: "hello".to_string(),
                    tool_name: None,
                    tool_call_id: None,
                    created_at: "2026-02-27T00:00:01Z".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"session_history""#));
        assert!(json.contains(r#""session_id":"session-a""#));
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            WsMessage::SessionHistory {
                session_id,
                messages,
            } => {
                assert_eq!(session_id, "session-a");
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].role, "user");
                assert_eq!(messages[1].role, "assistant");
            }
            _ => panic!("expected SessionHistory"),
        }
    }

    #[test]
    fn ws_message_session_history_request_roundtrip() {
        let msg = WsMessage::SessionHistoryRequest {
            session_id: "session-b".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"session_history_request""#));
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            WsMessage::SessionHistoryRequest { session_id } => {
                assert_eq!(session_id, "session-b");
            }
            _ => panic!("expected SessionHistoryRequest"),
        }
    }

    #[test]
    fn to_web_history_filters_system_messages_and_preserves_order() {
        use proto::{AgentMessage, Role};
        let session_id = proto::SessionId::from("s1");
        let messages = vec![
            AgentMessage::new(session_id.clone(), Role::User, "hello"),
            AgentMessage::new(session_id.clone(), Role::System, "system prompt"),
            AgentMessage::new(session_id.clone(), Role::Assistant, "hi there"),
        ];
        let history = to_web_history(messages);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "hello");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "hi there");
    }

    #[test]
    fn with_session_loader_sets_loader() {
        struct DummyLoader;
        #[async_trait]
        impl SessionLoader for DummyLoader {
            async fn load_session_messages(
                &self,
                _session_id: &str,
            ) -> Result<Vec<proto::AgentMessage>, String> {
                Ok(vec![])
            }
        }

        let adapter = WebAdapter::new(
            3210,
            "token".to_string(),
            "*".to_string(),
            "".to_string(),
            "shared-main".to_string(),
        )
        .with_session_loader(Arc::new(DummyLoader));

        assert!(adapter.session_loader.is_some());
    }

    #[test]
    fn ws_message_model_list_request_roundtrip() {
        let json = serde_json::to_string(&WsMessage::ModelListRequest).expect("serialize");
        assert!(json.contains(r#""type":"model_list_request""#));
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(parsed, WsMessage::ModelListRequest));
    }

    #[test]
    fn ws_message_model_list_roundtrip() {
        let msg = WsMessage::ModelList {
            models: vec![WebModelEntry {
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                recommended: true,
            }],
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"model_list""#));
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            WsMessage::ModelList { models } => {
                assert_eq!(models.len(), 1);
                assert_eq!(models[0].provider, "openai");
                assert_eq!(models[0].model, "gpt-4o");
                assert!(models[0].recommended);
            }
            _ => panic!("expected ModelList"),
        }
    }

    #[test]
    fn ws_message_model_change_roundtrip() {
        let msg = WsMessage::ModelChange {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-6".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"model_change""#));
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            WsMessage::ModelChange { provider, model } => {
                assert_eq!(provider, "anthropic");
                assert_eq!(model, "claude-sonnet-4-6");
            }
            _ => panic!("expected ModelChange"),
        }
    }

    #[test]
    fn ws_message_model_changed_roundtrip() {
        let msg = WsMessage::ModelChanged {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"model_changed""#));
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        match parsed {
            WsMessage::ModelChanged { provider, model } => {
                assert_eq!(provider, "openai");
                assert_eq!(model, "gpt-4o");
            }
            _ => panic!("expected ModelChanged"),
        }
    }

    #[test]
    fn web_model_entry_serializes_correctly() {
        let entry = WebModelEntry {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            recommended: true,
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        assert!(json.contains(r#""recommended":true"#));
        let parsed: WebModelEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, entry);
    }

    #[test]
    fn with_model_list_sets_catalog() {
        let adapter = WebAdapter::new(
            3210,
            "token".to_string(),
            "*".to_string(),
            "".to_string(),
            "shared-main".to_string(),
        )
        .with_model_list(vec![WebModelEntry {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            recommended: true,
        }]);
        assert_eq!(adapter.model_list.len(), 1);
    }

    #[test]
    fn with_model_change_callback_sets_callback() {
        let adapter = WebAdapter::new(
            3210,
            "token".to_string(),
            "*".to_string(),
            "".to_string(),
            "shared-main".to_string(),
        )
        .with_model_change_callback(Arc::new(|_p, _m| {}));
        assert!(adapter.model_change_cb.is_some());
    }

    #[test]
    fn ws_message_cancel_generation_roundtrip() {
        let json = serde_json::to_string(&WsMessage::CancelGeneration).expect("serialize");
        assert!(json.contains(r#""type":"cancel_generation""#));
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(parsed, WsMessage::CancelGeneration));
    }

    #[test]
    fn ws_message_generation_cancelled_roundtrip() {
        let json = serde_json::to_string(&WsMessage::GenerationCancelled).expect("serialize");
        assert!(json.contains(r#""type":"generation_cancelled""#));
        let parsed: WsMessage = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(parsed, WsMessage::GenerationCancelled));
    }
}
