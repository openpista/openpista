use thiserror::Error;

/// Top-level error type
#[derive(Debug, Error)]
pub enum Error {
    /// Configuration loading/validation error.
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),

    /// Gateway transport/runtime error.
    #[error("Gateway error: {0}")]
    Gateway(#[from] GatewayError),

    /// LLM provider error.
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    /// Tool registration/execution error.
    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    /// Database/migration error.
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    /// Channel adapter error.
    #[error("Channel error: {0}")]
    Channel(#[from] ChannelError),

    /// Internal protocol type error.
    #[error("Proto error: {0}")]
    Proto(#[from] ProtoError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Configuration errors
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A required field was not provided.
    #[error("Missing required field: {0}")]
    MissingField(String),

    /// A field has an invalid value and reason.
    #[error("Invalid value for {field}: {reason}")]
    InvalidValue { field: String, reason: String },

    /// Filesystem read error.
    #[error("IO error reading config: {0}")]
    Io(#[from] std::io::Error),

    /// TOML parse error.
    #[error("TOML parse error: {0}")]
    Toml(String),
}

/// Gateway errors
#[derive(Debug, Error)]
pub enum GatewayError {
    /// Network/connection-level failure.
    #[error("Connection error: {0}")]
    Connection(String),

    /// TLS setup/handshake failure.
    #[error("TLS error: {0}")]
    Tls(String),

    /// Session lookup failure.
    #[error("Session not found: {0}")]
    SessionNotFound(String),
}

/// LLM provider errors
#[derive(Debug, Error)]
pub enum LlmError {
    /// Remote API failure.
    #[error("{0}")]
    Api(String),

    /// Provider throttled the request.
    #[error("Rate limit exceeded")]
    RateLimit,

    /// Provider response schema/content was invalid.
    #[error("Invalid response from LLM: {0}")]
    InvalidResponse(String),

    /// Runtime exceeded configured tool-call rounds.
    #[error("Max tool rounds exceeded")]
    MaxToolRoundsExceeded,

    /// Serialization/deserialization failure.
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Tool execution errors
#[derive(Debug, Error)]
pub enum ToolError {
    /// Requested tool is unknown.
    #[error("Tool not found: {0}")]
    NotFound(String),

    /// Tool process or operation failed.
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    /// Tool exceeded allowed execution time.
    #[error("Timeout after {0}s")]
    Timeout(u64),

    /// Tool call arguments are invalid.
    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),

    /// Filesystem/process IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Database errors
#[derive(Debug, Error)]
pub enum DatabaseError {
    /// SQLx operation error.
    #[error("SQLx error: {0}")]
    Sqlx(String),

    /// Migration execution error.
    #[error("Migration error: {0}")]
    Migration(String),

    /// Requested record was not found.
    #[error("Not found: {0}")]
    NotFound(String),
}

/// Channel adapter errors
#[derive(Debug, Error)]
pub enum ChannelError {
    /// Channel connection failed.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Sending message/event failed.
    #[error("Send failed: {0}")]
    SendFailed(String),

    /// Adapter authentication failed.
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    /// Channel has been closed.
    #[error("Channel closed")]
    Closed,
}

/// Internal proto errors
#[derive(Debug, Error)]
pub enum ProtoError {
    /// Invalid role string value.
    #[error("Invalid role: {0}")]
    InvalidRole(String),

    /// Generic serialization/deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_config_error_variant() {
        let err = ConfigError::MissingField("agent.model".to_string());
        assert!(err.to_string().contains("Missing required field"));
    }

    #[test]
    fn wraps_gateway_error_into_top_level_error() {
        let err: Error = GatewayError::Connection("closed".to_string()).into();
        assert!(err.to_string().contains("Gateway error"));
    }

    #[test]
    fn wraps_llm_error_into_top_level_error() {
        let err: Error = LlmError::MaxToolRoundsExceeded.into();
        assert!(err.to_string().contains("Max tool rounds exceeded"));
    }

    #[test]
    fn wraps_tool_and_channel_errors() {
        let tool_err: Error = ToolError::InvalidArgs("missing command".to_string()).into();
        assert!(tool_err.to_string().contains("Tool error"));

        let channel_err: Error = ChannelError::Closed.into();
        assert!(channel_err.to_string().contains("Channel error"));
    }

    #[test]
    fn wraps_database_and_proto_errors() {
        let db_err: Error = DatabaseError::NotFound("session".to_string()).into();
        assert!(db_err.to_string().contains("Database error"));

        let proto_err: Error = ProtoError::InvalidRole("owner".to_string()).into();
        assert!(proto_err.to_string().contains("Proto error"));
    }
}
