//! Telegram channel adapter implementation.

use async_trait::async_trait;
use proto::{AgentResponse, ChannelError, ChannelEvent, ChannelId, SessionId};
use teloxide::{dispatching::UpdateFilterExt, prelude::*, types::Message};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::adapter::ChannelAdapter;

/// Telegram adapter using teloxide
pub struct TelegramAdapter {
    bot: Bot,
    channel_prefix: String,
}

impl TelegramAdapter {
    /// Creates a new Telegram adapter from a bot token.
    pub fn new(token: impl Into<String>) -> Self {
        let bot = Bot::new(token.into());
        Self {
            bot,
            channel_prefix: "telegram".to_string(),
        }
    }

    /// Creates a stable session id for a Telegram chat id.
    fn make_session_id(chat_id: i64) -> SessionId {
        SessionId::from(format!("telegram:{chat_id}"))
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel_id(&self) -> ChannelId {
        ChannelId::new(&self.channel_prefix, "bot")
    }

    async fn run(self, tx: mpsc::Sender<ChannelEvent>) -> Result<(), ChannelError> {
        info!("Telegram adapter starting");

        let handler = Update::filter_message().branch(
            dptree::filter(|msg: Message| msg.text().is_some()).endpoint(
                move |_bot: Bot, msg: Message, tx: mpsc::Sender<ChannelEvent>| async move {
                    let text = msg.text().unwrap_or("").to_string();
                    let chat_id = msg.chat.id.0;
                    let event = build_channel_event(chat_id, text);

                    if let Err(e) = tx.send(event).await {
                        error!("Failed to send Telegram event: {e}");
                    }

                    respond(())
                },
            ),
        );

        Dispatcher::builder(self.bot, handler)
            .dependencies(dptree::deps![tx])
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;

        info!("Telegram dispatcher stopped");
        Ok(())
    }

    async fn send_response(&self, resp: AgentResponse) -> Result<(), ChannelError> {
        let chat_id = parse_chat_id(resp.channel_id.as_str())?;
        let text = format_response_text(&resp);

        self.bot
            .send_message(ChatId(chat_id), text)
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;

        Ok(())
    }
}

/// Builds a channel event from Telegram chat id and message text.
fn build_channel_event(chat_id: i64, text: String) -> ChannelEvent {
    let channel_id = ChannelId::new("telegram", &chat_id.to_string());
    let session_id = TelegramAdapter::make_session_id(chat_id);
    ChannelEvent::new(channel_id, session_id, text)
}

/// Parses a Telegram chat id from `telegram:<id>` or raw numeric string.
fn parse_chat_id(channel_str: &str) -> Result<i64, ChannelError> {
    let chat_id_str = channel_str.strip_prefix("telegram:").unwrap_or(channel_str);
    chat_id_str.parse().map_err(|e| {
        ChannelError::SendFailed(format!("Invalid Telegram chat_id '{chat_id_str}': {e}"))
    })
}

/// Formats Telegram response text with error marker when needed.
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
    fn make_session_id_uses_telegram_prefix() {
        let sid = TelegramAdapter::make_session_id(42);
        assert_eq!(sid.as_str(), "telegram:42");
    }

    #[test]
    fn parse_chat_id_accepts_prefixed_and_raw_values() {
        assert_eq!(parse_chat_id("telegram:123").expect("prefixed id"), 123);
        assert_eq!(parse_chat_id("456").expect("raw id"), 456);
    }

    #[test]
    fn parse_chat_id_rejects_invalid_values() {
        let err = parse_chat_id("telegram:abc").expect_err("invalid id should fail");
        assert!(err.to_string().contains("Invalid Telegram chat_id"));
    }

    #[test]
    fn format_response_text_marks_errors() {
        let ok = AgentResponse::new(ChannelId::from("telegram:1"), SessionId::from("s1"), "ok");
        assert_eq!(format_response_text(&ok), "ok");

        let err =
            AgentResponse::error(ChannelId::from("telegram:1"), SessionId::from("s1"), "boom");
        assert!(format_response_text(&err).starts_with("❌ Error: "));
    }

    #[test]
    fn build_channel_event_populates_ids() {
        let event = build_channel_event(99, "hello".to_string());
        assert_eq!(event.channel_id.as_str(), "telegram:99");
        assert_eq!(event.session_id.as_str(), "telegram:99");
        assert_eq!(event.user_message, "hello");
    }

    #[test]
    fn adapter_new_creates_with_telegram_prefix() {
        let adapter = TelegramAdapter::new("fake-token");
        assert_eq!(adapter.channel_prefix, "telegram");
    }

    #[test]
    fn adapter_channel_id_returns_telegram_bot() {
        let adapter = TelegramAdapter::new("fake-token");
        let cid = adapter.channel_id();
        assert_eq!(cid.as_str(), "telegram:bot");
    }

    #[test]
    fn parse_chat_id_handles_negative_ids() {
        assert_eq!(
            parse_chat_id("telegram:-100123").expect("negative id"),
            -100123
        );
        assert_eq!(parse_chat_id("-999").expect("raw negative"), -999);
    }

    #[test]
    fn format_response_text_preserves_content_exactly() {
        let resp = AgentResponse::new(
            ChannelId::from("telegram:1"),
            SessionId::from("s1"),
            "hello world",
        );
        assert_eq!(format_response_text(&resp), "hello world");
    }

    #[test]
    fn build_channel_event_with_negative_chat_id() {
        let event = build_channel_event(-100, "msg".to_string());
        assert_eq!(event.channel_id.as_str(), "telegram:-100");
        assert_eq!(event.session_id.as_str(), "telegram:-100");
    }

    #[test]
    fn make_session_id_handles_zero() {
        let sid = TelegramAdapter::make_session_id(0);
        assert_eq!(sid.as_str(), "telegram:0");
    }
}
