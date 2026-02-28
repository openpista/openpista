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
}
