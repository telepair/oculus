//! User-facing storage facades.
//!
//! This module provides ergonomic APIs for interacting with the storage layer:
//! - Writers: Async facades that send commands via MPSC channel
//! - Readers: Sync facades that query via r2d2 pool

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::storage::StorageError;
use crate::storage::actor::Command;
use crate::storage::pool::ReadPool;
use crate::storage::types::{Event, EventType, Metric, Severity};

// =============================================================================
// Query Types
// =============================================================================

/// Sort order for query results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    /// Ascending order (oldest first).
    Asc,
    /// Descending order (newest first). This is the default.
    #[default]
    Desc,
}

impl SortOrder {
    /// Returns the SQL keyword for this sort order.
    fn as_sql(&self) -> &'static str {
        match self {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        }
    }
}

// =============================================================================
// Query Constants
// =============================================================================

/// Default limit for query results.
const DEFAULT_QUERY_LIMIT: u32 = 100;

/// Maximum limit for query results.
const MAX_QUERY_LIMIT: u32 = 10_000;

/// Default time range in days (for start time when not specified).
const DEFAULT_TIME_RANGE_DAYS: i64 = 7;

// =============================================================================
// Query Structs
// =============================================================================

/// Query parameters for metrics.
///
/// # Example
/// ```ignore
/// let results = reader.query(MetricQuery {
///     symbol: Some("btc.price".to_string()),
///     limit: Some(50),
///     ..Default::default()
/// })?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct MetricQuery {
    /// Start time filter (default: 7 days ago).
    pub start: Option<DateTime<Utc>>,
    /// End time filter (default: now).
    pub end: Option<DateTime<Utc>>,
    /// Category filter.
    pub category: Option<String>,
    /// Symbol filter.
    pub symbol: Option<String>,
    /// Maximum number of results (default: 100, max: 10,000).
    pub limit: Option<u32>,
    /// Sort order (default: Desc).
    pub order: Option<SortOrder>,
}

/// Query parameters for events.
///
/// # Example
/// ```ignore
/// let results = reader.query(EventQuery {
///     source: Some("system".to_string()),
///     severity: Some(Severity::Error),
///     ..Default::default()
/// })?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    /// Start time filter (default: 7 days ago).
    pub start: Option<DateTime<Utc>>,
    /// End time filter (default: now).
    pub end: Option<DateTime<Utc>>,
    /// Source filter.
    pub source: Option<String>,
    /// Event type filter.
    pub event_type: Option<EventType>,
    /// Severity filter.
    pub severity: Option<Severity>,
    /// Maximum number of results (default: 100, max: 10,000).
    pub limit: Option<u32>,
    /// Sort order (default: Desc).
    pub order: Option<SortOrder>,
}

// =============================================================================
// Writers
// =============================================================================

/// Facade for writing metrics.
#[derive(Clone)]
pub struct MetricWriter {
    tx: SyncSender<Command>,
}

impl MetricWriter {
    /// Create a new metric writer.
    pub(crate) fn new(tx: SyncSender<Command>) -> Self {
        Self { tx }
    }

    /// Insert a single metric.
    pub fn insert(&self, metric: Metric) -> Result<(), StorageError> {
        self.tx
            .send(Command::InsertMetric(metric))
            .map_err(|_| StorageError::ChannelSend)
    }

    /// Insert a batch of metrics.
    pub fn insert_batch(&self, metrics: Vec<Metric>) -> Result<(), StorageError> {
        self.tx
            .send(Command::InsertMetrics(metrics))
            .map_err(|_| StorageError::ChannelSend)
    }
}

/// Facade for writing events.
#[derive(Clone)]
pub struct EventWriter {
    tx: SyncSender<Command>,
}

impl EventWriter {
    /// Create a new event writer.
    pub(crate) fn new(tx: SyncSender<Command>) -> Self {
        Self { tx }
    }

    /// Insert a single event.
    pub fn insert(&self, event: Event) -> Result<(), StorageError> {
        self.tx
            .send(Command::InsertEvent(event))
            .map_err(|_| StorageError::ChannelSend)
    }

    /// Insert a batch of events.
    pub fn insert_batch(&self, events: Vec<Event>) -> Result<(), StorageError> {
        self.tx
            .send(Command::InsertEvents(events))
            .map_err(|_| StorageError::ChannelSend)
    }
}

// =============================================================================
// Readers
// =============================================================================

/// Facade for reading metrics.
#[derive(Clone)]
pub struct MetricReader {
    pool: Arc<ReadPool>,
}

impl MetricReader {
    /// Create a new metric reader.
    pub(crate) fn new(pool: Arc<ReadPool>) -> Self {
        Self { pool }
    }

    /// Query metrics with optional filters.
    ///
    /// # Defaults
    /// - `start`: 7 days ago if not specified
    /// - `end`: now if not specified
    /// - `limit`: 100 if not specified, capped at 10,000
    /// - `order`: Descending (newest first) if not specified
    pub fn query(&self, q: MetricQuery) -> Result<Vec<Metric>, StorageError> {
        let conn = self.pool.get()?;

        // Apply defaults for time range
        let now = Utc::now();
        let effective_end = q.end.unwrap_or(now);
        let effective_start = q
            .start
            .unwrap_or_else(|| now - Duration::days(DEFAULT_TIME_RANGE_DAYS));

        // Apply defaults and cap for limit
        let effective_limit = q.limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT);
        let effective_order = q.order.unwrap_or_default();

        let mut sql =
            String::from("SELECT ts, category, symbol, value, tags FROM metrics WHERE 1=1");
        let mut params: Vec<Box<dyn duckdb::ToSql>> = Vec::new();

        sql.push_str(" AND ts >= ?");
        params.push(Box::new(effective_start.timestamp_micros()));

        sql.push_str(" AND ts <= ?");
        params.push(Box::new(effective_end.timestamp_micros()));

        if let Some(ref c) = q.category {
            sql.push_str(" AND category = ?");
            params.push(Box::new(c.clone()));
        }
        if let Some(ref s) = q.symbol {
            sql.push_str(" AND symbol = ?");
            params.push(Box::new(s.clone()));
        }

        sql.push_str(&format!(" ORDER BY ts {}", effective_order.as_sql()));
        sql.push_str(&format!(" LIMIT {effective_limit}"));

        let param_refs: Vec<&dyn duckdb::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let ts_micros: i64 = row.get(0)?;
            let ts = DateTime::from_timestamp_micros(ts_micros).unwrap_or(DateTime::UNIX_EPOCH);
            let category: String = row.get(1)?;
            let symbol: String = row.get(2)?;
            let value: f64 = row.get(3)?;
            let tags_str: Option<String> = row.get(4)?;
            let tags = tags_str.and_then(|s| serde_json::from_str(&s).ok());

            Ok(Metric {
                ts,
                category,
                symbol,
                value,
                tags,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }
}

/// Facade for reading events.
#[derive(Clone)]
pub struct EventReader {
    pool: Arc<ReadPool>,
}

impl EventReader {
    /// Create a new event reader.
    pub(crate) fn new(pool: Arc<ReadPool>) -> Self {
        Self { pool }
    }

    /// Query events with optional filters.
    ///
    /// # Defaults
    /// - `start`: 7 days ago if not specified
    /// - `end`: now if not specified
    /// - `limit`: 100 if not specified, capped at 10,000
    /// - `order`: Descending (newest first) if not specified
    pub fn query(&self, q: EventQuery) -> Result<Vec<Event>, StorageError> {
        let conn = self.pool.get()?;

        // Apply defaults for time range
        let now = Utc::now();
        let effective_end = q.end.unwrap_or(now);
        let effective_start = q
            .start
            .unwrap_or_else(|| now - Duration::days(DEFAULT_TIME_RANGE_DAYS));

        // Apply defaults and cap for limit
        let effective_limit = q.limit.unwrap_or(DEFAULT_QUERY_LIMIT).min(MAX_QUERY_LIMIT);
        let effective_order = q.order.unwrap_or_default();

        // Cast ENUM columns to VARCHAR for Rust driver compatibility
        let mut sql = String::from(
            "SELECT id, ts, source, type::VARCHAR, severity::VARCHAR, message, payload FROM events WHERE 1=1",
        );
        let mut params: Vec<Box<dyn duckdb::ToSql>> = Vec::new();

        sql.push_str(" AND ts >= ?");
        params.push(Box::new(effective_start.timestamp_micros()));

        sql.push_str(" AND ts <= ?");
        params.push(Box::new(effective_end.timestamp_micros()));

        if let Some(ref s) = q.source {
            sql.push_str(" AND source = ?");
            params.push(Box::new(s.clone()));
        }
        if let Some(t) = q.event_type {
            sql.push_str(" AND type = ?");
            params.push(Box::new(t.as_str().to_string()));
        }
        if let Some(s) = q.severity {
            sql.push_str(" AND severity = ?");
            params.push(Box::new(s.as_str().to_string()));
        }

        sql.push_str(&format!(" ORDER BY ts {}", effective_order.as_sql()));
        sql.push_str(&format!(" LIMIT {effective_limit}"));

        let param_refs: Vec<&dyn duckdb::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let id: Option<i64> = row.get(0)?;
            let ts_micros: i64 = row.get(1)?;
            let ts = DateTime::from_timestamp_micros(ts_micros).unwrap_or(DateTime::UNIX_EPOCH);
            let source: String = row.get(2)?;
            let type_str: String = row.get(3)?;
            let severity_str: String = row.get(4)?;
            let message: String = row.get(5)?;
            let payload_str: Option<String> = row.get(6)?;
            let payload = payload_str.and_then(|s| serde_json::from_str(&s).ok());

            Ok(Event {
                id,
                ts,
                source,
                event_type: EventType::from_str(&type_str).unwrap_or(EventType::System),
                severity: Severity::from_str(&severity_str).unwrap_or(Severity::Info),
                message,
                payload,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }
}

/// Facade for executing raw SQL queries.
#[derive(Clone)]
pub struct RawSqlReader {
    pool: Arc<ReadPool>,
}

impl RawSqlReader {
    /// Create a new raw SQL reader.
    pub(crate) fn new(pool: Arc<ReadPool>) -> Self {
        Self { pool }
    }

    /// Execute a raw SQL query and return results as key-value maps.
    ///
    /// Column names are extracted by wrapping the query in a DESCRIBE statement.
    pub fn execute(&self, sql: &str) -> Result<Vec<HashMap<String, Value>>, StorageError> {
        let conn = self.pool.get()?;

        // Get column names by wrapping query in a subquery and using DESCRIBE
        // This is a workaround for DuckDB 1.4.2's column_name API requiring execution
        let describe_sql = format!("DESCRIBE SELECT * FROM ({sql}) AS _q");
        let column_names: Vec<String> = {
            let mut desc_stmt = conn.prepare(&describe_sql)?;
            let mut desc_rows = desc_stmt.query([])?;
            let mut names = Vec::new();
            while let Some(row) = desc_rows.next()? {
                let name: String = row.get(0)?;
                names.push(name);
            }
            names
        };

        let mut stmt = conn.prepare(sql)?;
        let mut rows_iter = stmt.query([])?;
        let mut results = Vec::new();

        while let Some(row) = rows_iter.next()? {
            let mut map = HashMap::new();
            for (i, name) in column_names.iter().enumerate() {
                // Try to get value as different types
                let value: Value = if let Ok(v) = row.get::<_, i64>(i) {
                    Value::Number(v.into())
                } else if let Ok(v) = row.get::<_, f64>(i) {
                    serde_json::Number::from_f64(v)
                        .map(Value::Number)
                        .unwrap_or(Value::Null)
                } else if let Ok(v) = row.get::<_, String>(i) {
                    Value::String(v)
                } else if let Ok(v) = row.get::<_, bool>(i) {
                    Value::Bool(v)
                } else {
                    Value::Null
                };
                map.insert(name.clone(), value);
            }
            results.push(map);
        }

        Ok(results)
    }
}

// =============================================================================
// Admin
// =============================================================================

/// Facade for storage administration operations.
#[derive(Clone)]
pub struct StorageAdmin {
    tx: SyncSender<Command>,
}

impl StorageAdmin {
    /// Create a new storage admin.
    pub(crate) fn new(tx: SyncSender<Command>) -> Self {
        Self { tx }
    }

    /// Delete metrics older than retention_days.
    pub fn cleanup_metrics(&self, retention_days: u32) -> Result<(), StorageError> {
        self.tx
            .send(Command::CleanupMetrics { retention_days })
            .map_err(|_| StorageError::ChannelSend)
    }

    /// Delete events older than retention_days.
    pub fn cleanup_events(&self, retention_days: u32) -> Result<(), StorageError> {
        self.tx
            .send(Command::CleanupEvents { retention_days })
            .map_err(|_| StorageError::ChannelSend)
    }

    /// Force WAL checkpoint for read visibility.
    pub fn checkpoint(&self) -> Result<(), StorageError> {
        self.tx
            .send(Command::Checkpoint)
            .map_err(|_| StorageError::ChannelSend)
    }

    /// Graceful shutdown of the writer actor.
    pub fn shutdown(&self) -> Result<(), StorageError> {
        self.tx
            .send(Command::Shutdown)
            .map_err(|_| StorageError::ChannelSend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::actor::DbActor;
    use chrono::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_metric_writer_and_reader() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Start actor
        let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();

        // Create facades
        let writer = MetricWriter::new(tx.clone());
        let admin = StorageAdmin::new(tx);

        // Write metric
        let metric = Metric {
            ts: Utc::now(),
            category: "test".to_string(),
            symbol: "test.metric".to_string(),
            value: 42.0,
            tags: None,
        };
        writer.insert(metric).unwrap();

        // Checkpoint and shutdown
        admin.checkpoint().unwrap();
        admin.shutdown().unwrap();
        handle.join().unwrap();

        // Create reader pool and verify
        let pool = ReadPool::new(&db_path, 2).unwrap();
        let reader = MetricReader::new(pool);

        let results = reader.query(MetricQuery::default()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol, "test.metric");
    }

    #[test]
    fn test_event_writer_and_reader() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("events.db");

        // Phase 1: Write events
        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = EventWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let event = Event {
                id: None,
                ts: Utc::now(),
                source: "test.source".to_string(),
                event_type: EventType::Alert,
                severity: Severity::Warn,
                message: "Test alert message".to_string(),
                payload: Some(serde_json::json!({"key": "value"})),
            };
            writer.insert(event).unwrap();

            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        // Phase 2: Read events
        let pool = ReadPool::new(&db_path, 2).unwrap();
        let reader = EventReader::new(pool);

        let results = reader.query(EventQuery::default()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "test.source");
        assert_eq!(results[0].event_type, EventType::Alert);
        assert_eq!(results[0].severity, Severity::Warn);
        assert!(results[0].id.is_some());
    }

    #[test]
    fn test_event_batch_insert() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("events_batch.db");

        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = EventWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let events: Vec<Event> = (0..5)
                .map(|i| Event {
                    id: None,
                    ts: Utc::now(),
                    source: format!("source.{i}"),
                    event_type: EventType::System,
                    severity: Severity::Info,
                    message: format!("Event {i}"),
                    payload: None,
                })
                .collect();

            writer.insert_batch(events).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        let pool = ReadPool::new(&db_path, 2).unwrap();
        let reader = EventReader::new(pool);
        let results = reader.query(EventQuery::default()).unwrap();
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_raw_sql_reader() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("raw_sql.db");

        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = MetricWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            for i in 0..3 {
                let metric = Metric {
                    ts: Utc::now(),
                    category: "raw".to_string(),
                    symbol: format!("raw.metric.{i}"),
                    value: f64::from(i * 10),
                    tags: None,
                };
                writer.insert(metric).unwrap();
            }

            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        let pool = ReadPool::new(&db_path, 2).unwrap();
        let raw_reader = RawSqlReader::new(pool);

        let results = raw_reader
            .execute("SELECT symbol, value FROM metrics WHERE category = 'raw' ORDER BY value")
            .unwrap();
        assert_eq!(results.len(), 3);

        // Check first row
        let first = &results[0];
        assert!(
            first
                .get("symbol")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("raw.metric")
        );
    }

    #[test]
    fn test_metric_query_filters() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("filters.db");

        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = MetricWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            // Insert metrics with different categories and symbols
            let metrics = vec![
                Metric {
                    ts: Utc::now(),
                    category: "crypto".to_string(),
                    symbol: "btc.price".to_string(),
                    value: 100000.0,
                    tags: None,
                },
                Metric {
                    ts: Utc::now(),
                    category: "crypto".to_string(),
                    symbol: "eth.price".to_string(),
                    value: 4000.0,
                    tags: None,
                },
                Metric {
                    ts: Utc::now(),
                    category: "network".to_string(),
                    symbol: "ping.latency".to_string(),
                    value: 50.0,
                    tags: None,
                },
            ];
            writer.insert_batch(metrics).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        let pool = ReadPool::new(&db_path, 2).unwrap();
        let reader = MetricReader::new(pool);

        // Test category filter
        let results = reader
            .query(MetricQuery {
                category: Some("crypto".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 2);

        // Test symbol filter
        let results = reader
            .query(MetricQuery {
                symbol: Some("btc.price".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, 100000.0);
    }

    #[test]
    fn test_event_query_filters() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("event_filters.db");

        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = EventWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let events = vec![
                Event {
                    id: None,
                    ts: Utc::now(),
                    source: "rule.btc".to_string(),
                    event_type: EventType::Alert,
                    severity: Severity::Critical,
                    message: "BTC alert".to_string(),
                    payload: None,
                },
                Event {
                    id: None,
                    ts: Utc::now(),
                    source: "collector.net".to_string(),
                    event_type: EventType::Error,
                    severity: Severity::Error,
                    message: "Network error".to_string(),
                    payload: None,
                },
                Event {
                    id: None,
                    ts: Utc::now(),
                    source: "system".to_string(),
                    event_type: EventType::System,
                    severity: Severity::Info,
                    message: "System startup".to_string(),
                    payload: None,
                },
            ];
            writer.insert_batch(events).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        let pool = ReadPool::new(&db_path, 2).unwrap();
        let reader = EventReader::new(pool);

        // Test source filter
        let results = reader
            .query(EventQuery {
                source: Some("rule.btc".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message, "BTC alert");

        // Test event_type filter
        let results = reader
            .query(EventQuery {
                event_type: Some(EventType::Error),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "collector.net");

        // Test severity filter
        let results = reader
            .query(EventQuery {
                severity: Some(Severity::Critical),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_sort_order() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("sort.db");

        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = MetricWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            // Insert metrics at different times
            let now = Utc::now();
            let metrics = vec![
                Metric {
                    ts: now - Duration::hours(2),
                    category: "sort".to_string(),
                    symbol: "oldest".to_string(),
                    value: 1.0,
                    tags: None,
                },
                Metric {
                    ts: now - Duration::hours(1),
                    category: "sort".to_string(),
                    symbol: "middle".to_string(),
                    value: 2.0,
                    tags: None,
                },
                Metric {
                    ts: now,
                    category: "sort".to_string(),
                    symbol: "newest".to_string(),
                    value: 3.0,
                    tags: None,
                },
            ];
            writer.insert_batch(metrics).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        let pool = ReadPool::new(&db_path, 2).unwrap();
        let reader = MetricReader::new(pool);

        // Test DESC (default) - newest first
        let results = reader
            .query(MetricQuery {
                category: Some("sort".to_string()),
                order: Some(SortOrder::Desc),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results[0].symbol, "newest");
        assert_eq!(results[2].symbol, "oldest");

        // Test ASC - oldest first
        let results = reader
            .query(MetricQuery {
                category: Some("sort".to_string()),
                order: Some(SortOrder::Asc),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results[0].symbol, "oldest");
        assert_eq!(results[2].symbol, "newest");
    }

    #[test]
    fn test_limit_cap() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("limit.db");

        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = MetricWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            // Insert more than the default limit but less than max
            let metrics: Vec<Metric> = (0..150)
                .map(|i| Metric {
                    ts: Utc::now(),
                    category: "limit".to_string(),
                    symbol: format!("metric.{i}"),
                    value: f64::from(i),
                    tags: None,
                })
                .collect();
            writer.insert_batch(metrics).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        let pool = ReadPool::new(&db_path, 2).unwrap();
        let reader = MetricReader::new(pool);

        // Default limit (100)
        let results = reader
            .query(MetricQuery {
                category: Some("limit".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 100);

        // Custom limit
        let results = reader
            .query(MetricQuery {
                category: Some("limit".to_string()),
                limit: Some(50),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 50);

        // Get all
        let results = reader
            .query(MetricQuery {
                category: Some("limit".to_string()),
                limit: Some(200),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 150);
    }

    #[test]
    fn test_time_range_filter() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("time_range.db");

        let now = Utc::now();

        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = MetricWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let metrics = vec![
                Metric {
                    ts: now - Duration::days(10), // Outside default range
                    category: "time".to_string(),
                    symbol: "old".to_string(),
                    value: 1.0,
                    tags: None,
                },
                Metric {
                    ts: now - Duration::days(3), // Within default range
                    category: "time".to_string(),
                    symbol: "recent".to_string(),
                    value: 2.0,
                    tags: None,
                },
                Metric {
                    ts: now,
                    category: "time".to_string(),
                    symbol: "now".to_string(),
                    value: 3.0,
                    tags: None,
                },
            ];
            writer.insert_batch(metrics).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        let pool = ReadPool::new(&db_path, 2).unwrap();
        let reader = MetricReader::new(pool);

        // Default time range (7 days) should exclude 10-day old metric
        let results = reader
            .query(MetricQuery {
                category: Some("time".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 2);

        // Explicit wide time range should include all
        let results = reader
            .query(MetricQuery {
                category: Some("time".to_string()),
                start: Some(now - Duration::days(30)),
                end: Some(now + Duration::hours(1)),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 3);
    }
}
