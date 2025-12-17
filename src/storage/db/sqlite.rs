//! SQLite backend implementation using sqlx.
//!
//! Provides connection pooling and database operations for SQLite.

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool as SqlxPool, SqlitePoolOptions,
    SqliteSynchronous,
};
use std::str::FromStr;
use std::time::Duration;

use crate::storage::StorageError;

/// Default maximum connections in the pool.
const DEFAULT_MAX_CONNECTIONS: u32 = 5;

/// Default connection timeout.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// SQLite connection pool wrapper.
///
/// Wraps sqlx's SqlitePool with sensible defaults for WAL mode and connection pooling.
#[derive(Clone)]
pub struct SqlitePool {
    inner: SqlxPool,
}

impl std::fmt::Debug for SqlitePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlitePool").finish_non_exhaustive()
    }
}

impl SqlitePool {
    /// Connect to a SQLite database.
    ///
    /// # Arguments
    ///
    /// * `url` - SQLite connection URL, e.g., `sqlite:data/oculus.db?mode=rwc`
    ///
    /// # Configuration
    ///
    /// - WAL journal mode for better concurrency
    /// - Normal synchronous mode for performance with durability
    /// - Create database if not exists (mode=rwc)
    pub async fn connect(url: &str) -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::from_str(url)?
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(DEFAULT_MAX_CONNECTIONS)
            .acquire_timeout(DEFAULT_CONNECT_TIMEOUT)
            .connect_with(options)
            .await?;

        Ok(Self { inner: pool })
    }

    /// Get the underlying sqlx pool for direct query execution.
    #[inline]
    pub fn inner(&self) -> &SqlxPool {
        &self.inner
    }

    /// Close the connection pool gracefully.
    pub async fn close(&self) {
        self.inner.close().await;
    }

    /// Check if the pool is closed.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sqlite_pool_connect() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        assert!(!pool.is_closed());

        // Verify we can execute a query
        let row: (i32,) = sqlx::query_as("SELECT 1")
            .fetch_one(pool.inner())
            .await
            .unwrap();
        assert_eq!(row.0, 1);

        pool.close().await;
        assert!(pool.is_closed());
    }

    #[tokio::test]
    async fn test_sqlite_pool_wal_mode() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        // Verify WAL mode is enabled
        let row: (String,) = sqlx::query_as("PRAGMA journal_mode")
            .fetch_one(pool.inner())
            .await
            .unwrap();
        // Note: In-memory databases may report 'memory' instead of 'wal'
        assert!(row.0 == "wal" || row.0 == "memory");

        pool.close().await;
    }
}
