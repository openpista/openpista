//! In-process gateway helpers.

use std::sync::Arc;

use proto::ChannelEvent;
use tokio::sync::mpsc;

/// Async callback that processes inbound [`ChannelEvent`] and returns optional text.
pub type AgentHandler = Arc<
    dyn Fn(
            ChannelEvent,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>>
        + Send
        + Sync,
>;

/// Simple in-process "gateway" using tokio channels (for CLI/testing)
pub struct InProcessGateway {
    tx: mpsc::Sender<ChannelEvent>,
    rx: mpsc::Receiver<ChannelEvent>,
}

impl InProcessGateway {
    /// Creates a bounded in-process gateway.
    pub fn new(buffer: usize) -> Self {
        let (tx, rx) = mpsc::channel(buffer);
        Self { tx, rx }
    }

    /// Returns a cloneable sender used to enqueue inbound events.
    pub fn sender(&self) -> mpsc::Sender<ChannelEvent> {
        self.tx.clone()
    }

    /// Receives the next inbound event from the queue.
    pub async fn recv(&mut self) -> Option<ChannelEvent> {
        self.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_handler() -> AgentHandler {
        Arc::new(|_event| Box::pin(async move { Some("ok".to_string()) }))
    }

    #[tokio::test]
    async fn in_process_gateway_forwards_events() {
        let mut gateway = InProcessGateway::new(4);
        let sender = gateway.sender();
        let event = ChannelEvent::new(
            proto::ChannelId::from("cli:local"),
            proto::SessionId::from("s1"),
            "hello",
        );
        sender.send(event.clone()).await.expect("send should work");

        let received = gateway.recv().await.expect("event should be received");
        assert_eq!(received.channel_id, event.channel_id);
        assert_eq!(received.session_id, event.session_id);
        assert_eq!(received.user_message, "hello");
    }

    #[test]
    fn noop_handler_compiles() {
        let _handler = noop_handler();
    }
}
