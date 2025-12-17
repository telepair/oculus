//! Storage-specific error types.
//!
//! All storage operations return [`StorageError`] on failure, which can be
//! matched to determine the underlying cause (database, pool, channel, etc.).

use thiserror::Error;

/// Errors that can occur in the storage layer.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Database operation failed (sqlx error).
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Failed to send command to writer actor.
    #[error("failed to send command to writer actor")]
    ChannelSend,

    /// JSON serialization/deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Internal error (e.g., task join failure).
    #[error("internal error: {0}")]
    Internal(String),

    /// Invalid data in database (e.g., unknown enum value).
    #[error("invalid data: {0}")]
    InvalidData(String),

    /// Migration error.
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}
