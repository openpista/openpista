//! WhatsApp access-token-based channel adapter implementation.

use async_trait::async_trait;
use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::adapter::ChannelAdapter;

/// Configuration for the WhatsApp adapter.
///
/// Mirrors `WhatsAppConfig` from `crates/cli/src/config.rs` to avoid a
/// reverse dependency from channels → cli.
#[derive(Debug, Clone)]
pub struct WhatsAppAdapterConfig {
    /// WhatsApp phone number (e.g. `15551234567`).
    pub phone_number: String,
    /// Access token for the WhatsApp gateway.
    pub access_token: String,
    /// HTTP port for the webhook server.
    pub webhook_port: u16,
}

/// WhatsApp access-token-based adapter.
///
/// Receives inbound messages via a webhook and sends
/// outbound messages via the Graph API using Bearer auth.
pub struct WhatsAppAdapter {
    access_token: String,
    phone_number: String,
    webhook_port: u16,
    http: reqwest::Client,
    #[allow(dead_code)]
    resp_tx: mpsc::Sender<AgentResponse>,
}

impl WhatsAppAdapter {
    /// Creates a new WhatsApp adapter from config and response channel.
    pub fn new(config: WhatsAppAdapterConfig, resp_tx: mpsc::Sender<AgentResponse>) -> Self {
        Self {
            access_token: config.access_token,
            phone_number: config.phone_number,
            webhook_port: config.webhook_port,
            http: reqwest::Client::new(),
            resp_tx,
        }
    }

    /// Creates a stable session id for a WhatsApp phone number.
    fn make_session_id(phone: &str) -> SessionId {
        SessionId::from(format!("whatsapp:{phone}"))
    }
}

// ─── Axum shared state ─────────────────────────────────────

#[derive(Clone)]
struct WhatsAppState {
    access_token: String,
    event_tx: mpsc::Sender<ChannelEvent>,
}

// ─── Webhook verification query params ─────────────────────

#[derive(Deserialize)]
struct VerifyQuery {
    #[serde(rename = "hub.mode")]
    hub_mode: Option<String>,
    #[serde(rename = "hub.challenge")]
    hub_challenge: Option<String>,
}

// ─── WhatsApp webhook payload types ────────────────────────

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    #[allow(dead_code)]
    object: Option<String>,
    entry: Option<Vec<WebhookEntry>>,
}

#[derive(Debug, Deserialize)]
struct WebhookEntry {
    changes: Option<Vec<WebhookChange>>,
}

#[derive(Debug, Deserialize)]
struct WebhookChange {
    value: Option<WebhookValue>,
}

#[derive(Debug, Deserialize)]
struct WebhookValue {
    messages: Option<Vec<WebhookMessage>>,
}

#[derive(Debug, Deserialize)]
struct WebhookMessage {
    from: Option<String>,
    #[serde(rename = "type")]
    msg_type: Option<String>,
    text: Option<WebhookText>,
}

#[derive(Debug, Deserialize)]
struct WebhookText {
    body: Option<String>,
}

// ─── Graph API send body ───────────────────────────────────

#[derive(Serialize)]
struct SendMessageBody {
    messaging_product: String,
    to: String,
    #[serde(rename = "type")]
    msg_type: String,
    text: SendText,
}

#[derive(Serialize)]
struct SendText {
    body: String,
}

// ─── ChannelAdapter impl ───────────────────────────────────

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    fn channel_id(&self) -> ChannelId {
        ChannelId::new("whatsapp", "webhook")
    }

    async fn run(self, tx: mpsc::Sender<ChannelEvent>) -> Result<(), ChannelError> {
        info!("WhatsApp adapter starting on port {}", self.webhook_port);

        let state = WhatsAppState {
            access_token: self.access_token.clone(),
            event_tx: tx,
        };

        let app = Router::new()
            .route("/webhook", get(webhook_verify))
            .route("/webhook", post(webhook_receive))
            .with_state(state);

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], self.webhook_port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("bind failed: {e}")))?;

        info!("WhatsApp webhook listening on {addr}");
        axum::serve(listener, app)
            .await
            .map_err(|e| ChannelError::ConnectionFailed(format!("server error: {e}")))?;

        info!("WhatsApp adapter stopped");
        Ok(())
    }

    async fn send_response(&self, resp: AgentResponse) -> Result<(), ChannelError> {
        let phone = parse_phone_from_channel_id(resp.channel_id.as_str())?;
        let text = format_response_text(&resp);

        let body = SendMessageBody {
            messaging_product: "whatsapp".to_string(),
            to: phone,
            msg_type: "text".to_string(),
            text: SendText { body: text },
        };

        let url = format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.phone_number
        );

        self.http
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(format!("Graph API error: {e}")))?;

        Ok(())
    }
}

// ─── Axum handlers ─────────────────────────────────────────

/// GET /webhook — simple verification challenge (no verify_token needed).
async fn webhook_verify(
    State(_state): State<WhatsAppState>,
    Query(params): Query<VerifyQuery>,
) -> impl IntoResponse {
    if let (Some(mode), Some(challenge)) = (&params.hub_mode, &params.hub_challenge)
        && mode == "subscribe"
    {
        debug!("WhatsApp webhook verified");
        return (StatusCode::OK, challenge.clone());
    }
    (StatusCode::FORBIDDEN, "Verification failed".to_string())
}

/// POST /webhook — incoming message with Bearer access_token verification.
async fn webhook_receive(
    State(state): State<WhatsAppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Bearer token verification
    if !state.access_token.is_empty() {
        let auth_ok = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|token| token == state.access_token);
        if !auth_ok {
            warn!("WhatsApp webhook: invalid or missing Bearer token");
            return StatusCode::UNAUTHORIZED;
        }
    }

    // Parse payload
    let payload: WebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to parse WhatsApp webhook payload: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Extract text messages (ignore status updates)
    let messages = extract_messages(&payload);
    for (phone, text) in messages {
        let channel_id = ChannelId::new("whatsapp", &phone);
        let session_id = WhatsAppAdapter::make_session_id(&phone);
        let event = ChannelEvent::new(channel_id, session_id, text);

        if let Err(e) = state.event_tx.send(event).await {
            error!("Failed to forward WhatsApp event: {e}");
        }
    }

    StatusCode::OK
}

// ─── Helpers ───────────────────────────────────────────────

/// Extracts `(phone, text)` pairs from a WhatsApp webhook payload.
/// Ignores status updates (entries without `messages`).
fn extract_messages(payload: &WebhookPayload) -> Vec<(String, String)> {
    let mut messages = Vec::new();
    let Some(entries) = &payload.entry else {
        return messages;
    };
    for entry in entries {
        let Some(changes) = &entry.changes else {
            continue;
        };
        for change in changes {
            let Some(value) = &change.value else {
                continue;
            };
            let Some(msgs) = &value.messages else {
                continue;
            };
            for msg in msgs {
                if let (Some(from), Some("text"), Some(text)) =
                    (&msg.from, msg.msg_type.as_deref(), &msg.text)
                    && let Some(body) = &text.body
                {
                    messages.push((from.clone(), body.clone()));
                }
            }
        }
    }
    messages
}

/// Parses a phone number from `whatsapp:<phone>` or raw string.
fn parse_phone_from_channel_id(channel_str: &str) -> Result<String, ChannelError> {
    let phone = channel_str.strip_prefix("whatsapp:").unwrap_or(channel_str);
    if phone.is_empty() {
        Err(ChannelError::SendFailed(
            "Empty phone number in channel id".to_string(),
        ))
    } else {
        Ok(phone.to_string())
    }
}

/// Formats response text with error marker when needed.
fn format_response_text(resp: &AgentResponse) -> String {
    if resp.is_error {
        format!("❌ Error: {}", resp.content)
    } else {
        resp.content.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_session_id_uses_whatsapp_prefix() {
        let sid = WhatsAppAdapter::make_session_id("15551234567");
        assert_eq!(sid.as_str(), "whatsapp:15551234567");
    }

    #[test]
    fn parse_phone_handles_prefixed_and_raw() {
        assert_eq!(
            parse_phone_from_channel_id("whatsapp:15551234567").unwrap(),
            "15551234567"
        );
        assert_eq!(
            parse_phone_from_channel_id("15551234567").unwrap(),
            "15551234567"
        );
    }

    #[test]
    fn parse_phone_rejects_empty() {
        assert!(parse_phone_from_channel_id("whatsapp:").is_err());
    }

    #[test]
    fn format_response_text_marks_errors() {
        let ok = AgentResponse::new(ChannelId::from("whatsapp:1"), SessionId::from("s1"), "ok");
        assert_eq!(format_response_text(&ok), "ok");

        let err =
            AgentResponse::error(ChannelId::from("whatsapp:1"), SessionId::from("s1"), "boom");
        assert!(format_response_text(&err).starts_with("❌ Error: "));
    }

    #[test]
    fn extract_messages_parses_valid_payload() {
        let json = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "123",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "messages": [{
                            "from": "15551234567",
                            "id": "wamid.xxx",
                            "timestamp": "1234567890",
                            "type": "text",
                            "text": {"body": "Hello"}
                        }]
                    },
                    "field": "messages"
                }]
            }]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        let messages = extract_messages(&payload);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, "15551234567");
        assert_eq!(messages[0].1, "Hello");
    }

    #[test]
    fn extract_messages_ignores_status_updates() {
        let json = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "changes": [{
                    "value": {},
                    "field": "messages"
                }]
            }]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        let messages = extract_messages(&payload);
        assert!(messages.is_empty());
    }

    #[test]
    fn extract_messages_ignores_non_text_messages() {
        let json = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "15551234567",
                            "type": "image"
                        }]
                    }
                }]
            }]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        let messages = extract_messages(&payload);
        assert!(messages.is_empty());
    }

    #[test]
    fn extract_messages_handles_multiple_messages() {
        let json = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [
                            {"from": "111", "type": "text", "text": {"body": "A"}},
                            {"from": "222", "type": "text", "text": {"body": "B"}}
                        ]
                    }
                }]
            }]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        let messages = extract_messages(&payload);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], ("111".to_string(), "A".to_string()));
        assert_eq!(messages[1], ("222".to_string(), "B".to_string()));
    }
}
