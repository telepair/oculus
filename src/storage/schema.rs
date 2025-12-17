//! Database schema definitions and migrations.
//!
//! SQLite-compatible schema for metrics, events, and collectors.
//! Uses TEXT with CHECK constraints instead of ENUM types.

use sqlx::SqlitePool;

use crate::storage::StorageError;

/// SQL statement for creating the metric_series table (dimension table).
///
/// Primary key is the hash-based series_id for deduplication.
/// Uses INSERT OR REPLACE for upsert logic.
pub const METRIC_SERIES_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS metric_series (
    series_id   INTEGER PRIMARY KEY,
    category    TEXT NOT NULL CHECK(category IN (
        'network.tcp', 'network.ping', 'network.http',
        'crypto', 'polymarket', 'stock', 'custom'
    )),
    name        TEXT NOT NULL,
    target      TEXT NOT NULL,
    static_tags TEXT DEFAULT '{}',
    description TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
"#;

/// SQL statement for creating the metric_values table (data table).
///
/// Stores time-series data points linked to series via series_id.
pub const METRIC_VALUES_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS metric_values (
    ts           INTEGER NOT NULL,
    series_id    INTEGER NOT NULL,
    value        REAL NOT NULL,
    unit         TEXT,
    success      INTEGER NOT NULL DEFAULT 1,
    duration_ms  INTEGER,
    dynamic_tags TEXT DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_metric_values_ts ON metric_values(ts);
CREATE INDEX IF NOT EXISTS idx_metric_values_series_id ON metric_values(series_id);
"#;

/// SQL statement for creating the events table.
///
/// Uses TEXT fields with CHECK constraints for enum-like validation.
pub const EVENTS_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS events (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    ts        INTEGER NOT NULL,
    source    TEXT NOT NULL CHECK(source IN (
        'collector.network.tcp', 'collector.network.ping', 'collector.network.http',
        'rule.engine', 'system'
    )),
    kind      TEXT NOT NULL CHECK(kind IN ('alert', 'error', 'system', 'audit')),
    severity  TEXT NOT NULL CHECK(severity IN ('debug', 'info', 'warn', 'error', 'critical')),
    message   TEXT NOT NULL,
    payload   TEXT DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
CREATE INDEX IF NOT EXISTS idx_events_source ON events(source);
"#;

/// SQL statement for creating the collectors table.
///
/// Stores collector configurations with source tracking.
pub const COLLECTORS_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS collectors (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    type        TEXT NOT NULL CHECK(type IN ('tcp', 'ping', 'http')),
    name        TEXT NOT NULL,
    source      TEXT NOT NULL CHECK(source IN ('config', 'api')),
    enabled     INTEGER NOT NULL DEFAULT 1,
    group_name  TEXT NOT NULL DEFAULT 'default',
    config      TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    UNIQUE(type, name)
);
"#;

/// Initialize the database schema.
///
/// Creates all necessary tables if they don't exist.
pub async fn init_schema(pool: &SqlitePool) -> Result<(), StorageError> {
    sqlx::query(METRIC_SERIES_TABLE_DDL).execute(pool).await?;
    sqlx::query(METRIC_VALUES_TABLE_DDL).execute(pool).await?;
    sqlx::query(EVENTS_TABLE_DDL).execute(pool).await?;
    sqlx::query(COLLECTORS_TABLE_DDL).execute(pool).await?;

    tracing::info!("Database schema initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::SqlitePool;

    #[tokio::test]
    async fn test_schema_initialization() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_schema(pool.inner()).await.unwrap();

        // Verify metric_series table exists
        let count: (i32,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='metric_series'",
        )
        .fetch_one(pool.inner())
        .await
        .unwrap();
        assert_eq!(count.0, 1);

        // Verify metric_values table exists
        let count: (i32,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='metric_values'",
        )
        .fetch_one(pool.inner())
        .await
        .unwrap();
        assert_eq!(count.0, 1);

        // Verify events table exists
        let count: (i32,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='events'",
        )
        .fetch_one(pool.inner())
        .await
        .unwrap();
        assert_eq!(count.0, 1);

        // Verify collectors table exists
        let count: (i32,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='collectors'",
        )
        .fetch_one(pool.inner())
        .await
        .unwrap();
        assert_eq!(count.0, 1);

        pool.close().await;
    }

    #[tokio::test]
    async fn test_metric_series_upsert() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_schema(pool.inner()).await.unwrap();

        // Insert a series
        sqlx::query(
            "INSERT INTO metric_series (series_id, category, name, target, created_at, updated_at)
             VALUES (12345, 'network.tcp', 'latency', '127.0.0.1:6379', 1000, 1000)",
        )
        .execute(pool.inner())
        .await
        .unwrap();

        // Upsert with same series_id should replace
        sqlx::query(
            "INSERT OR REPLACE INTO metric_series (series_id, category, name, target, description, created_at, updated_at)
             VALUES (12345, 'network.tcp', 'latency', '127.0.0.1:6379', 'Updated desc', 1000, 2000)",
        )
        .execute(pool.inner())
        .await
        .unwrap();

        // Verify only one row exists
        let count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM metric_series")
            .fetch_one(pool.inner())
            .await
            .unwrap();
        assert_eq!(count.0, 1);

        // Verify description was updated
        let desc: (String,) =
            sqlx::query_as("SELECT description FROM metric_series WHERE series_id = 12345")
                .fetch_one(pool.inner())
                .await
                .unwrap();
        assert_eq!(desc.0, "Updated desc");

        pool.close().await;
    }
}
