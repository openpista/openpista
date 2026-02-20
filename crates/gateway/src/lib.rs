//! Gateway components for routing channel events and QUIC transport.

pub mod router;
pub mod scheduler;
pub mod server;
pub mod session;

/// Event router for channel/session mappings.
pub use router::ChannelRouter;
/// Cron scheduler wrapper for generating channel events.
pub use scheduler::CronScheduler;
/// QUIC server and in-process test gateway utilities.
pub use server::{AgentHandler, InProcessGateway, QuicServer};
/// QUIC session manager.
pub use session::AgentSession;
