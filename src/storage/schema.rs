//! Database schema definitions and migrations.

use duckdb::Connection;

use crate::storage::StorageError;

/// SQL statement for creating the metrics table.
///
/// Note: Timestamps are stored as BIGINT (microseconds since Unix epoch)
/// for reliable cross-connection queries.
pub const METRICS_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS metrics (
    ts          BIGINT      NOT NULL,
    category    VARCHAR     NOT NULL,
    symbol      VARCHAR     NOT NULL,
    value       DOUBLE      NOT NULL,
    tags        VARCHAR,
    success     BOOLEAN     NOT NULL DEFAULT TRUE,
    duration_ms BIGINT      NOT NULL DEFAULT 0
);
"#;

pub const EVENTS_ENUMS_DDL: &str = r#"
CREATE TYPE IF NOT EXISTS event_kind AS ENUM ('alert', 'error', 'system', 'audit');
CREATE TYPE IF NOT EXISTS event_severity AS ENUM ('debug', 'info', 'warn', 'error', 'critical');
"#;

/// SQL statement for creating the events table (using VARCHAR for type/severity for compatibility).
pub const EVENTS_TABLE_DDL: &str = r#"
CREATE SEQUENCE IF NOT EXISTS events_id_seq;
CREATE TABLE IF NOT EXISTS events (
    id        BIGINT      PRIMARY KEY DEFAULT NEXTVAL('events_id_seq'),
    ts        BIGINT      NOT NULL,
    source    VARCHAR     NOT NULL,
    kind      event_kind   NOT NULL,
    severity  event_severity NOT NULL,
    message   VARCHAR     NOT NULL,
    payload   VARCHAR
);
"#;

/// Initialize the database schema.
///
/// This creates all necessary tables and indexes if they don't exist.
pub fn init_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(METRICS_TABLE_DDL)?;
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

        // Verify tables exist
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'metrics'")
            .unwrap();
        let count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);

        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'events'")
            .unwrap();
        let count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }
}
