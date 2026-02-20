use dashmap::DashMap;
use proto::{AgentResponse, ChannelEvent, ChannelId, SessionId};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Routes channel events to the appropriate agent session
pub struct ChannelRouter {
    /// channel_id -> event sender
    channels: DashMap<ChannelId, mpsc::Sender<ChannelEvent>>,
    /// channel_id -> session_id mapping
    sessions: DashMap<ChannelId, SessionId>,
    /// response sender (for routing responses back to channels)
    response_tx: mpsc::Sender<AgentResponse>,
}

impl ChannelRouter {
    /// Creates a new router that forwards agent responses to `response_tx`.
    pub fn new(response_tx: mpsc::Sender<AgentResponse>) -> Self {
        Self {
            channels: DashMap::new(),
            sessions: DashMap::new(),
            response_tx,
        }
    }

    /// Register a channel adapter
    pub fn register(&self, channel_id: ChannelId, tx: mpsc::Sender<ChannelEvent>) {
        debug!("Registering channel: {channel_id}");
        self.channels.insert(channel_id, tx);
    }

    /// Deregister a channel adapter
    pub fn deregister(&self, channel_id: &ChannelId) {
        debug!("Deregistering channel: {channel_id}");
        self.channels.remove(channel_id);
        self.sessions.remove(channel_id);
    }

    /// Associate a channel with a session
    pub fn bind_session(&self, channel_id: ChannelId, session_id: SessionId) {
        self.sessions.insert(channel_id, session_id);
    }

    /// Get the session ID for a channel
    pub fn session_for(&self, channel_id: &ChannelId) -> Option<SessionId> {
        self.sessions.get(channel_id).map(|v| v.clone())
    }

    /// Route an event to its target channel's agent
    pub async fn route(&self, event: ChannelEvent) -> bool {
        if let Some(tx) = self.channels.get(&event.channel_id) {
            match tx.send(event).await {
                Ok(_) => true,
                Err(e) => {
                    warn!("Failed to route event: {e}");
                    false
                }
            }
        } else {
            warn!("No channel registered for: {}", event.channel_id);
            false
        }
    }

    /// Send a response back (to be picked up by channel adapters)
    pub async fn respond(&self, response: AgentResponse) {
        if let Err(e) = self.response_tx.send(response).await {
            warn!("Failed to send response: {e}");
        }
    }

    /// Number of registered channels
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event(channel: &str, session: &str, message: &str) -> ChannelEvent {
        ChannelEvent::new(ChannelId::from(channel), SessionId::from(session), message)
    }

    #[tokio::test]
    async fn register_and_route_event_successfully() {
        let (resp_tx, _resp_rx) = mpsc::channel(4);
        let router = ChannelRouter::new(resp_tx);
        let (tx, mut rx) = mpsc::channel(4);
        let channel_id = ChannelId::from("cli:local");

        router.register(channel_id.clone(), tx);
        assert_eq!(router.channel_count(), 1);

        let ok = router.route(sample_event("cli:local", "s1", "hello")).await;
        assert!(ok);

        let received = rx.recv().await.expect("event should be routed");
        assert_eq!(received.channel_id, channel_id);
        assert_eq!(received.user_message, "hello");
    }

    #[tokio::test]
    async fn route_returns_false_when_channel_missing() {
        let (resp_tx, _resp_rx) = mpsc::channel(2);
        let router = ChannelRouter::new(resp_tx);

        let ok = router
            .route(sample_event("cli:missing", "s1", "ping"))
            .await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn bind_session_lookup_and_deregister() {
        let (resp_tx, _resp_rx) = mpsc::channel(2);
        let router = ChannelRouter::new(resp_tx);
        let (tx, _rx) = mpsc::channel(2);
        let channel = ChannelId::from("telegram:7");
        let session = SessionId::from("session-7");

        router.register(channel.clone(), tx);
        router.bind_session(channel.clone(), session.clone());
        assert_eq!(router.session_for(&channel), Some(session));

        router.deregister(&channel);
        assert_eq!(router.channel_count(), 0);
        assert_eq!(router.session_for(&channel), None);
    }

    #[tokio::test]
    async fn respond_forwards_agent_response() {
        let (resp_tx, mut resp_rx) = mpsc::channel(2);
        let router = ChannelRouter::new(resp_tx);
        let response =
            AgentResponse::new(ChannelId::from("cli:local"), SessionId::from("s1"), "ok");

        router.respond(response.clone()).await;
        let forwarded = resp_rx.recv().await.expect("response should be forwarded");
        assert_eq!(forwarded.channel_id, response.channel_id);
        assert_eq!(forwarded.session_id, response.session_id);
        assert_eq!(forwarded.content, "ok");
    }
}
