//! Gateway components for routing channel events.

pub mod router;
pub mod scheduler;
pub mod server;

/// Event router for channel/session mappings.
pub use router::ChannelRouter;
/// Cron scheduler wrapper for generating channel events.
pub use scheduler::CronScheduler;
/// Agent handler type and in-process gateway utilities.
pub use server::{AgentHandler, InProcessGateway};
