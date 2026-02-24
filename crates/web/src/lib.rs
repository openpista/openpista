//! openpista-web: Rust→WASM browser client for the openpista Web adapter.
//!
//! Provides a [`Client`] that manages a WebSocket connection to the
//! openpista daemon, serialises messages as JSON, and persists the
//! session ID in `localStorage`.

use serde::Serialize;
use wasm_bindgen::prelude::*;

/// JSON frame sent over the WebSocket to the server.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outgoing_message_serializes_to_json() {
        let msg = OutgoingMessage {
            user_message: "hello world".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("user_message"));
        assert!(json.contains("hello world"));
    }

    #[test]
    fn outgoing_message_serializes_special_chars() {
        let msg = OutgoingMessage {
            user_message: "hello \"quotes\" & <angles>".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("hello"));
        // Verify it round-trips via serde_json
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
        assert!(json.contains("\"user_message\":\"\""));
    }

    // Tests below require wasm_bindgen types that only work on wasm32 targets.
    // Client::new, send, is_connected, connect, disconnect use JsValue/WebSocket
    // which panic in native test runners.
    #[cfg(target_arch = "wasm32")]
    mod wasm_only {
        use super::*;

        #[test]
        fn client_new_stores_url_and_token() {
            let client = Client::new("ws://localhost:3210/ws", "mytoken");
            assert_eq!(client.url, "ws://localhost:3210/ws");
            assert_eq!(client.token, "mytoken");
            assert!(client.ws.is_none());
        }

        #[test]
        fn client_new_empty_token() {
            let client = Client::new("ws://localhost:3210/ws", "");
            assert_eq!(client.token, "");
        }

        #[test]
        fn client_is_connected_returns_false_when_no_ws() {
            let client = Client::new("ws://localhost:3210/ws", "tok");
            assert!(
                !client.is_connected(),
                "should not be connected without a WebSocket"
            );
        }

        #[test]
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
