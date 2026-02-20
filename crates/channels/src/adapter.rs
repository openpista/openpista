//! Shared channel adapter trait.

use async_trait::async_trait;
use proto::{AgentResponse, ChannelEvent, ChannelId};
use tokio::sync::mpsc;

/// Trait for channel adapters (Telegram, CLI, Discord, etc.)
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// The unique identifier for this channel
    fn channel_id(&self) -> ChannelId;

    /// Run the adapter, publishing events to `tx`
    async fn run(self, tx: mpsc::Sender<ChannelEvent>) -> Result<(), proto::ChannelError>;

    /// Send a response back to the user
    async fn send_response(&self, resp: AgentResponse) -> Result<(), proto::ChannelError>;
}
