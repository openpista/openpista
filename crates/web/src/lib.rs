use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use web_sys::console;

// ─── WsMessage envelope (mirrors server) ───────────────────
//
// Kept for type-safe message serialization. As features migrate
// from app.js to WASM, this enum drives the WebSocket protocol.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    #[serde(rename = "message")]
    UserMessage { content: String },
    #[serde(rename = "response")]
    AgentReply { content: String, is_error: bool },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "auth")]
    Auth { token: String },
    #[serde(rename = "auth_result")]
    AuthResult {
        success: bool,
        client_id: Option<String>,
        error: Option<String>,
    },
}

// ─── Logging helper ────────────────────────────────────────

fn log(s: &str) {
    console::log_1(&JsValue::from_str(s));
}

// ─── Public API ────────────────────────────────────────────

/// Returns the crate version string.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// WASM entry point — called automatically by Trunk's JS glue.
///
/// Initialises panic hook for readable browser console errors and
/// logs the module version. All UI logic lives in app.js; the WASM
/// module only provides utilities (version, type-safe serialization).
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    log(&format!(
        "openpista-web v{} WASM module loaded",
        env!("CARGO_PKG_VERSION")
    ));
}

// ─── JSON frame sent over the WebSocket to the server ──────

#[derive(Serialize)]
struct OutgoingMessage {
    user_message: String,
}

/// WASM-exported WebSocket client for the openpista Web adapter.
#[wasm_bindgen]
pub struct Client {
    ws: Option<web_sys::WebSocket>,
    url: String,
    token: String,
}

#[wasm_bindgen]
impl Client {
    /// Creates a new client targeting `url` (e.g. `ws://localhost:3210/ws`).
    /// The optional `token` is appended as a `?token=` query parameter.
    #[wasm_bindgen(constructor)]
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            ws: None,
            url: url.to_string(),
            token: token.to_string(),
        }
    }

    /// Opens the WebSocket connection. Returns an error on failure.
    pub fn connect(&mut self) -> Result<(), JsValue> {
        let ws_url = if self.token.is_empty() {
            self.url.clone()
        } else {
            format!("{}?token={}", self.url, self.token)
        };

        let ws = web_sys::WebSocket::new(&ws_url)?;

        // Persist session id in localStorage
        let session_id = self.load_or_create_session_id();
        self.save_session_id(&session_id);

        self.ws = Some(ws);
        Ok(())
    }

    /// Sends a user message over the WebSocket as JSON.
    pub fn send(&self, message: &str) -> Result<(), JsValue> {
        let Some(ws) = &self.ws else {
            return Err(JsValue::from_str("Not connected"));
        };
        let payload = OutgoingMessage {
            user_message: message.to_string(),
        };
        let json = serde_json::to_string(&payload)
            .map_err(|e| JsValue::from_str(&format!("JSON error: {e}")))?;
        ws.send_with_str(&json)
    }

    /// Closes the WebSocket connection.
    pub fn disconnect(&mut self) {
        if let Some(ws) = self.ws.take() {
            let _ = ws.close();
        }
    }

    /// Returns `true` if the underlying WebSocket is currently open.
    pub fn is_connected(&self) -> bool {
        self.ws
            .as_ref()
            .is_some_and(|ws| ws.ready_state() == web_sys::WebSocket::OPEN)
    }
}

// ─── Private helpers (not exported to JS) ──────────────────

impl Client {
    fn load_or_create_session_id(&self) -> String {
        if let Some(storage) = self.get_storage()
            && let Ok(Some(id)) = storage.get_item("openpista_session_id")
        {
            return id;
        }
        let id = format!("web-{}", js_sys::Math::random().to_bits());
        id
    }

    fn save_session_id(&self, id: &str) {
        if let Some(storage) = self.get_storage() {
            let _ = storage.set_item("openpista_session_id", id);
        }
    }

    fn get_storage(&self) -> Option<web_sys::Storage> {
        let window = web_sys::window()?;
        window.local_storage().ok()?
    }
}

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_message_user_message_serialize() {
        let msg = WsMessage::UserMessage {
            content: "hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"message""#));
        assert!(json.contains(r#""content":"hello""#));
    }

    #[test]
    fn test_ws_message_agent_reply_deserialize() {
        let json = r#"{"type":"response","content":"hi","is_error":false}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsMessage::AgentReply { content, is_error } => {
                assert_eq!(content, "hi");
                assert!(!is_error);
            }
            _ => panic!("expected AgentReply"),
        }
    }

    #[test]
    fn test_ws_message_auth_result_deserialize() {
        let json = r#"{"type":"auth_result","success":true,"client_id":"abc","error":null}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsMessage::AuthResult {
                success,
                client_id,
                error,
            } => {
                assert!(success);
                assert_eq!(client_id.as_deref(), Some("abc"));
                assert!(error.is_none());
            }
            _ => panic!("expected AuthResult"),
        }
    }

    #[test]
    fn test_ws_message_ping_pong_roundtrip() {
        let ping_json = serde_json::to_string(&WsMessage::Ping).unwrap();
        let parsed: WsMessage = serde_json::from_str(&ping_json).unwrap();
        assert!(matches!(parsed, WsMessage::Ping));

        let pong_json = serde_json::to_string(&WsMessage::Pong).unwrap();
        let parsed: WsMessage = serde_json::from_str(&pong_json).unwrap();
        assert!(matches!(parsed, WsMessage::Pong));
    }

    #[test]
    fn test_ws_message_auth_serialize() {
        let msg = WsMessage::Auth {
            token: "secret".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"auth""#));
        assert!(json.contains(r#""token":"secret""#));
    }

    #[test]
    fn outgoing_message_serializes_special_chars() {
        let msg = OutgoingMessage {
            user_message: "hello \"quotes\" & <angles>".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert_eq!(json, r#"{"user_message":"hello \"quotes\" & <angles>"}"#);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(
            parsed["user_message"].as_str().unwrap(),
            "hello \"quotes\" & <angles>"
        );
    }

    #[test]
    fn outgoing_message_empty_string() {
        let msg = OutgoingMessage {
            user_message: String::new(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert_eq!(json, r#"{"user_message":""}"#);
    }

    #[cfg(target_arch = "wasm32")]
    pub mod wasm_only {
        use super::*;
        use wasm_bindgen_test::*;

        wasm_bindgen_test_configure!(run_in_browser);

        #[wasm_bindgen_test]
        fn client_new_stores_url_and_token() {
            let client = Client::new("ws://localhost:3210/ws", "mytoken");
            assert_eq!(client.url, "ws://localhost:3210/ws");
            assert_eq!(client.token, "mytoken");
            assert!(client.ws.is_none());
        }

        #[wasm_bindgen_test]
        fn client_new_empty_token() {
            let client = Client::new("ws://localhost:3210/ws", "");
            assert_eq!(client.token, "");
        }

        #[wasm_bindgen_test]
        fn client_is_connected_returns_false_when_no_ws() {
            let client = Client::new("ws://localhost:3210/ws", "tok");
            assert!(
                !client.is_connected(),
                "should not be connected without a WebSocket"
            );
        }

        #[wasm_bindgen_test]
        fn client_send_returns_error_when_not_connected() {
            let client = Client::new("ws://localhost:3210/ws", "tok");
            let err = client
                .send("hello")
                .expect_err("should fail when disconnected");
            let err_str = err.as_string().unwrap_or_default();
            assert!(err_str.contains("Not connected"));
        }
    }
}
