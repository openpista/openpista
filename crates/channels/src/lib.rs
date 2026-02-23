//! Channel adapter interfaces and built-in channel implementations.

pub mod adapter;
pub mod cli;
pub mod telegram;

/// Trait implemented by all channel adapters.
pub use adapter::ChannelAdapter;
/// Local CLI adapter implementation.
pub use cli::CliAdapter;
/// Telegram adapter implementation.
pub use telegram::TelegramAdapter;
