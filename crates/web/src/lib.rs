use serde::{Deserialize, Serialize};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use web_sys::{
    console, Document, Element, HtmlInputElement, MessageEvent, Storage, WebSocket, Window,
};

// ─── Storage key ───────────────────────────────────────────

const STORAGE_KEY: &str = "openpista:web:client_id";

// ─── WsMessage envelope (mirrors server) ───────────────────

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

// ─── Global WebSocket handle ───────────────────────────────

// Thread-local because WASM is single-threaded.
thread_local! {
    static SOCKET: std::cell::RefCell<Option<WebSocket>> = const { std::cell::RefCell::new(None) };
}

// ─── DOM helpers ───────────────────────────────────────────

fn window() -> Window {
    web_sys::window().expect("no global `window`")
}

fn document() -> Document {
    window().document().expect("no `document` on window")
}

fn get_element(id: &str) -> Element {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("element #{id} not found"))
}

fn local_storage() -> Storage {
    window()
        .local_storage()
        .expect("localStorage access failed")
        .expect("localStorage is null")
}

fn get_storage(key: &str) -> String {
    local_storage()
        .get_item(key)
        .unwrap_or(None)
        .unwrap_or_default()
}

fn set_storage(key: &str, value: &str) {
    let _ = local_storage().set_item(key, value);
}

/// Toggle the status badge between online and offline.
fn set_status(online: bool) {
    let badge = get_element("status");
    badge.set_text_content(Some(if online { "Online" } else { "Offline" }));

    let class_list = badge.class_list();
    if online {
        let _ = class_list.add_1("online");
        let _ = class_list.remove_1("offline");
    } else {
        let _ = class_list.add_1("offline");
        let _ = class_list.remove_1("online");
    }
}

/// Create a message div and append it to the messages container.
fn append_message(role: &str, text: &str, is_error: bool) {
    let doc = document();
    let node = doc.create_element("div").expect("create div");

    let class = if is_error {
        format!("msg {role} error")
    } else {
        format!("msg {role}")
    };
    node.set_class_name(&class);
    node.set_text_content(Some(text));

    let container = get_element("messages");
    let _ = container.append_child(&node);

    // Scroll to bottom.
    let el: &web_sys::HtmlElement = container.dyn_ref::<web_sys::HtmlElement>().unwrap();
    el.set_scroll_top(el.scroll_height());
}

/// Build the WebSocket URL from the current page location + token input.
fn ws_url() -> String {
    let location = window().location();
    let protocol = location.protocol().unwrap_or_default();
    let ws_proto = if protocol == "https:" { "wss" } else { "ws" };
    let host = location.host().unwrap_or_default();

    let token_el = get_element("token");
    let token_input: &HtmlInputElement = token_el
        .dyn_ref::<HtmlInputElement>()
        .expect("#token is not an input");
    let token = token_input.value();
    let token = token.trim();

    let client_id = get_storage(STORAGE_KEY);

    let encoded_token = js_sys::encode_uri_component(token);
    let encoded_client = js_sys::encode_uri_component(&client_id);

    format!("{ws_proto}://{host}/ws?token={encoded_token}&client_id={encoded_client}")
}

// ─── WebSocket connection ──────────────────────────────────

fn connect() {
    // Don't reconnect if already open.
    let already_open = SOCKET.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|ws| ws.ready_state() == WebSocket::OPEN)
    });
    if already_open {
        return;
    }

    let url = ws_url();
    log(&format!("Connecting to {url}"));

    let ws = match WebSocket::new(&url) {
        Ok(ws) => ws,
        Err(e) => {
            log(&format!("WebSocket::new failed: {e:?}"));
            set_status(false);
            append_message("agent", "Connection error.", true);
            return;
        }
    };

    // ── onopen ──
    let onopen = Closure::<dyn FnMut()>::new(move || {
        log("WebSocket opened");
        set_status(true);
        append_message("agent", "Connected.", false);
    });
    ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    // ── onclose ──
    let onclose = Closure::<dyn FnMut()>::new(move || {
        log("WebSocket closed");
        set_status(false);
        append_message("agent", "Disconnected.", false);
    });
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    // ── onerror ──
    let onerror = Closure::<dyn FnMut()>::new(move || {
        log("WebSocket error");
        set_status(false);
        append_message("agent", "Connection error.", true);
    });
    ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onerror.forget();

    // ── onmessage ──
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let data = match event.data().dyn_into::<js_sys::JsString>() {
            Ok(s) => String::from(s),
            Err(_) => {
                log("Received non-string WebSocket message");
                return;
            }
        };

        let msg: WsMessage = match serde_json::from_str(&data) {
            Ok(m) => m,
            Err(_) => {
                append_message("agent", "Received invalid JSON message.", true);
                return;
            }
        };

        match msg {
            WsMessage::AuthResult {
                success,
                client_id,
                error,
            } => {
                if !success {
                    let err_text = error.as_deref().unwrap_or("Authentication failed.");
                    append_message("agent", err_text, true);
                    return;
                }
                if let Some(id) = client_id {
                    set_storage(STORAGE_KEY, &id);
                }
            }
            WsMessage::AgentReply { content, is_error } => {
                append_message("agent", &content, is_error);
            }
            WsMessage::Pong => {
                // Silently ignore pong heartbeats.
            }
            _ => {
                // Ignore other message types on the client side.
            }
        }
    });
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    // Store the socket globally.
    SOCKET.with(|cell| {
        *cell.borrow_mut() = Some(ws);
    });
}

/// Send a user message over the WebSocket.
fn send_message(content: &str) {
    let is_open = SOCKET.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|ws| ws.ready_state() == WebSocket::OPEN)
    });

    if !is_open {
        append_message("agent", "Not connected.", true);
        return;
    }

    let msg = WsMessage::UserMessage {
        content: content.to_string(),
    };
    let json = serde_json::to_string(&msg).expect("serialize UserMessage");

    SOCKET.with(|cell| {
        if let Some(ws) = cell.borrow().as_ref() {
            if let Err(e) = ws.send_with_str(&json) {
                log(&format!("send failed: {e:?}"));
                append_message("agent", "Failed to send message.", true);
            }
        }
    });
}

// ─── Logging helper ────────────────────────────────────────

fn log(s: &str) {
    console::log_1(&JsValue::from_str(s));
}

// ─── Public API ────────────────────────────────────────────

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen(start)]
pub fn start() {
    log("openpista-web WASM client starting");

    // Initial status: offline.
    set_status(false);

    // ── Connect button click ──
    let connect_btn = get_element("connect");
    let on_connect = Closure::<dyn FnMut()>::new(move || {
        connect();
    });
    connect_btn
        .add_event_listener_with_callback("click", on_connect.as_ref().unchecked_ref())
        .expect("add click listener to #connect");
    on_connect.forget();

    // ── Form submit ──
    let composer = get_element("composer");
    let on_submit = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.prevent_default();

        let input_el = get_element("message");
        let input: &HtmlInputElement = input_el
            .dyn_ref::<HtmlInputElement>()
            .expect("#message is not an input");

        let text = input.value();
        let text = text.trim();
        if text.is_empty() {
            return;
        }

        send_message(text);
        append_message("user", text, false);
        input.set_value("");
    });
    composer
        .add_event_listener_with_callback("submit", on_submit.as_ref().unchecked_ref())
        .expect("add submit listener to #composer");
    on_submit.forget();

    log("openpista-web WASM client ready");
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
