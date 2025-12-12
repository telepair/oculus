//! Database schema definitions and migrations.

use duckdb::Connection;

use crate::storage::StorageError;

/// SQL statement for creating the metric category enum.
pub const METRIC_CATEGORY_ENUM_DDL: &str = r#"
CREATE TYPE IF NOT EXISTS metric_category_enum AS ENUM (
    'network.tcp', 'network.ping', 'network.http',
    'crypto', 'polymarket', 'stock', 'custom'
);
"#;

/// SQL statement for creating the metric_series table (dimension table).
///
/// Primary key is the hash-based series_id for deduplication.
/// Uses ON CONFLICT for upsert logic.
/// Note: static_tags stored as JSON string for prepared statement compatibility.
pub const METRIC_SERIES_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS metric_series (
    series_id   UBIGINT PRIMARY KEY,
    category    metric_category_enum NOT NULL,
    name        VARCHAR NOT NULL,
    target      VARCHAR NOT NULL,
    static_tags VARCHAR DEFAULT '{}',
    description VARCHAR,
    created_at  BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL
);
"#;

/// SQL statement for creating the metric_values table (data table).
///
/// Stores time-series data points linked to series via series_id.
/// Note: dynamic_tags stored as JSON string for prepared statement compatibility.
pub const METRIC_VALUES_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS metric_values (
    ts           BIGINT NOT NULL,
    series_id    UBIGINT NOT NULL,
    value        DOUBLE NOT NULL,
    unit         VARCHAR,
    success      BOOLEAN NOT NULL DEFAULT true,
    duration_ms  UINTEGER,
    dynamic_tags VARCHAR DEFAULT '{}'
);
"#;

/// SQL statement for creating event enums.
pub const EVENTS_ENUMS_DDL: &str = r#"
CREATE TYPE IF NOT EXISTS event_source_enum AS ENUM (
    'collector.registry', 'collector.network.tcp', 'collector.network.ping', 'collector.network.http',
    'rule.engine', 'system', 'other'
);
CREATE TYPE IF NOT EXISTS event_kind_enum AS ENUM ('alert', 'error', 'system', 'audit');
CREATE TYPE IF NOT EXISTS event_severity_enum AS ENUM ('debug', 'info', 'warn', 'error', 'critical');
"#;

/// SQL statement for creating the events table (wide table design).
///
/// Uses ENUM compression for metadata and JSON string for flexible payload.
pub const EVENTS_TABLE_DDL: &str = r#"
CREATE SEQUENCE IF NOT EXISTS events_id_seq;
CREATE TABLE IF NOT EXISTS events (
    id        BIGINT PRIMARY KEY DEFAULT NEXTVAL('events_id_seq'),
    ts        BIGINT NOT NULL,
    source    event_source_enum NOT NULL,
    kind      event_kind_enum NOT NULL,
    severity  event_severity_enum NOT NULL,
    message   VARCHAR NOT NULL,
    payload   VARCHAR DEFAULT '{}'
);
"#;

/// Initialize the database schema.
///
/// Creates all necessary tables and enums if they don't exist.
pub fn init_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(METRIC_CATEGORY_ENUM_DDL)?;
    conn.execute_batch(METRIC_SERIES_TABLE_DDL)?;
    conn.execute_batch(METRIC_VALUES_TABLE_DDL)?;
    conn.execute_batch(EVENTS_ENUMS_DDL)?;
    conn.execute_batch(EVENTS_TABLE_DDL)?;

    tracing::info!("Database schema initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_initialization() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        // Verify metric_series table exists
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'metric_series'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Verify metric_values table exists
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'metric_values'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Verify events table exists
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_metric_series_upsert() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        // Insert a series
        conn.execute(
            "INSERT INTO metric_series (series_id, category, name, target, created_at, updated_at)
             VALUES (12345, 'network.tcp', 'latency', '127.0.0.1:6379', 1000, 1000)",
            [],
        )
        .unwrap();

        // Upsert with same series_id should update
        conn.execute(
            "INSERT INTO metric_series (series_id, category, name, target, description, created_at, updated_at)
             VALUES (12345, 'network.tcp', 'latency', '127.0.0.1:6379', 'Updated desc', 1000, 2000)
             ON CONFLICT (series_id) DO UPDATE SET
                 updated_at = EXCLUDED.updated_at,
                 description = EXCLUDED.description",
            [],
        )
        .unwrap();

        // Verify only one row exists
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metric_series", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Verify description was updated
        let desc: String = conn
            .query_row(
                "SELECT description FROM metric_series WHERE series_id = 12345",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(desc, "Updated desc");
    }
}
