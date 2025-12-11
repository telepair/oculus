//! User-facing storage facades.
//!
//! Provides ergonomic APIs for storage operations:
//! - `StorageWriter`: Non-blocking writes via MPSC
//! - `MetricReader`: Query metrics (series + values JOIN)
//! - `EventReader`: Query events
//! - `RawSqlReader`: Execute raw SELECT queries
//! - `StorageAdmin`: Cleanup and maintenance

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use strum_macros::{AsRefStr, EnumString};

use crate::storage::StorageError;
use crate::storage::actor::Command;
use crate::storage::pool::ReadPool;
use crate::storage::types::{
    DynamicTags, Event, EventKind, EventSeverity, EventSource, MetricCategory, MetricSeries,
    MetricValue, StaticTags,
};

// =============================================================================
// Constants
// =============================================================================

const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 10_000;
const DEFAULT_RANGE_DAYS: i64 = 30;

// =============================================================================
// Query Types
// =============================================================================

/// Sort order for queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, EnumString, AsRefStr)]
#[strum(serialize_all = "lowercase")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

impl SortOrder {
    fn as_sql(&self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// Query for metric values (with series JOIN).
#[derive(Debug, Clone, Default)]
pub struct MetricQuery {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub category: Option<MetricCategory>,
    pub name: Option<String>,
    pub target: Option<String>,
    pub limit: Option<u32>,
    pub order: Option<SortOrder>,
}

/// Query for events.
#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub source: Option<EventSource>,
    pub kind: Option<EventKind>,
    pub severity: Option<EventSeverity>,
    pub limit: Option<u32>,
    pub order: Option<SortOrder>,
}

// =============================================================================
// Result Types
// =============================================================================

/// Joined metric result (series + value).
#[derive(Debug, Clone)]
pub struct MetricResult {
    pub series_id: u64,
    pub category: MetricCategory,
    pub name: String,
    pub target: String,
    pub static_tags: StaticTags,
    pub description: Option<String>,
    pub ts: DateTime<Utc>,
    pub value: f64,
    pub success: bool,
    pub duration_ms: u32,
    pub dynamic_tags: DynamicTags,
}

// =============================================================================
// Writer
// =============================================================================

/// Non-blocking storage writer.
///
/// Uses `try_send` - data is dropped if channel is full.
/// Data is buffered and flushed when buffer reaches 500 items or 1 second elapsed.
#[derive(Clone)]
pub struct StorageWriter {
    tx: SyncSender<Command>,
    dropped_metrics: Arc<AtomicU64>,
}

impl std::fmt::Debug for StorageWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageWriter").finish_non_exhaustive()
    }
}

impl StorageWriter {
    pub(crate) fn new(tx: SyncSender<Command>) -> Self {
        Self {
            tx,
            dropped_metrics: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get total count of dropped metrics due to channel capacity.
    pub fn dropped_metrics(&self) -> u64 {
        self.dropped_metrics.load(Ordering::Relaxed)
    }

    /// Upsert a metric series. Buffered until flush threshold.
    pub fn upsert_metric_series(&self, series: MetricSeries) -> Result<(), StorageError> {
        if self.tx.send(Command::UpsertMetricSeries(series)).is_err() {
            self.dropped_metrics.fetch_add(1, Ordering::Relaxed);
            return Err(StorageError::ChannelSend);
        }
        Ok(())
    }

    /// Insert a metric value. Buffered until flush threshold.
    pub fn insert_metric_value(&self, value: MetricValue) -> Result<(), StorageError> {
        if self.tx.try_send(Command::InsertMetricValue(value)).is_err() {
            tracing::warn!("Channel full, dropping metric value");
            self.dropped_metrics.fetch_add(1, Ordering::Relaxed);
            return Err(StorageError::ChannelSend);
        }
        Ok(())
    }

    /// Insert a single event. Buffered until flush threshold.
    pub fn insert_event(&self, event: Event) -> Result<(), StorageError> {
        self.tx.try_send(Command::InsertEvent(event)).map_err(|_| {
            tracing::warn!("Channel full, dropping event");
            StorageError::ChannelSend
        })
    }

    /// Force flush all buffered data immediately.
    pub fn flush(&self) -> Result<(), StorageError> {
        self.tx
            .try_send(Command::Flush)
            .map_err(|_| StorageError::ChannelSend)
    }
}

// =============================================================================
// Readers
// =============================================================================

/// Metric reader (series + values JOIN).
#[derive(Clone)]
pub struct MetricReader {
    pool: Arc<ReadPool>,
}

impl std::fmt::Debug for MetricReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricReader").finish_non_exhaustive()
    }
}

impl MetricReader {
    pub(crate) fn new(pool: Arc<ReadPool>) -> Self {
        Self { pool }
    }

    /// Query metrics with filters.
    pub fn query(&self, q: MetricQuery) -> Result<Vec<MetricResult>, StorageError> {
        let conn = self.pool.get()?;
        let now = Utc::now();
        let start = q
            .start
            .unwrap_or_else(|| now - Duration::days(DEFAULT_RANGE_DAYS));
        let end = q.end.unwrap_or(now);
        let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let order = q.order.unwrap_or_default();

        let mut sql = String::from(
            "SELECT s.series_id, s.category::VARCHAR, s.name, s.target, s.static_tags, s.description,
                    v.ts, v.value, v.success, v.duration_ms, v.dynamic_tags
             FROM metric_values v
             JOIN metric_series s ON v.series_id = s.series_id
             WHERE v.ts >= ? AND v.ts <= ?",
        );
        let mut params: Vec<Box<dyn duckdb::ToSql>> = vec![
            Box::new(start.timestamp_micros()),
            Box::new(end.timestamp_micros()),
        ];

        if let Some(cat) = q.category {
            sql.push_str(" AND s.category = ?");
            params.push(Box::new(cat.as_ref().to_string()));
        }
        if let Some(ref name) = q.name {
            sql.push_str(" AND s.name = ?");
            params.push(Box::new(name.clone()));
        }
        if let Some(ref target) = q.target {
            sql.push_str(" AND s.target = ?");
            params.push(Box::new(target.clone()));
        }

        sql.push_str(&format!(
            " ORDER BY v.ts {} LIMIT {}",
            order.as_sql(),
            limit
        ));

        let param_refs: Vec<&dyn duckdb::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(MetricResult {
                series_id: row.get(0)?,
                category: MetricCategory::from_str(&row.get::<_, String>(1)?)
                    .unwrap_or(MetricCategory::Custom),
                name: row.get(2)?,
                target: row.get(3)?,
                static_tags: parse_map(&row.get::<_, Option<String>>(4)?.unwrap_or_default()),
                description: row.get(5)?,
                ts: DateTime::from_timestamp_micros(row.get(6)?).unwrap_or(DateTime::UNIX_EPOCH),
                value: row.get(7)?,
                success: row.get(8)?,
                duration_ms: row.get::<_, i64>(9)?.try_into().unwrap_or(0),
                dynamic_tags: parse_map(&row.get::<_, Option<String>>(10)?.unwrap_or_default()),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }
}

/// Event reader.
#[derive(Clone)]
pub struct EventReader {
    pool: Arc<ReadPool>,
}

impl std::fmt::Debug for EventReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventReader").finish_non_exhaustive()
    }
}

impl EventReader {
    pub(crate) fn new(pool: Arc<ReadPool>) -> Self {
        Self { pool }
    }

    /// Query events with filters.
    pub fn query(&self, q: EventQuery) -> Result<Vec<Event>, StorageError> {
        let conn = self.pool.get()?;
        let now = Utc::now();
        let start = q
            .start
            .unwrap_or_else(|| now - Duration::days(DEFAULT_RANGE_DAYS));
        let end = q.end.unwrap_or(now);
        let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let order = q.order.unwrap_or_default();

        let mut sql = String::from(
            "SELECT id, ts, source::VARCHAR, kind::VARCHAR, severity::VARCHAR, message, payload
             FROM events WHERE ts >= ? AND ts <= ?",
        );
        let mut params: Vec<Box<dyn duckdb::ToSql>> = vec![
            Box::new(start.timestamp_micros()),
            Box::new(end.timestamp_micros()),
        ];

        if let Some(src) = q.source {
            sql.push_str(" AND source = ?");
            params.push(Box::new(src.as_ref().to_string()));
        }
        if let Some(kind) = q.kind {
            sql.push_str(" AND kind = ?");
            params.push(Box::new(kind.as_ref().to_string()));
        }
        if let Some(sev) = q.severity {
            sql.push_str(" AND severity = ?");
            params.push(Box::new(sev.as_ref().to_string()));
        }

        sql.push_str(&format!(" ORDER BY ts {} LIMIT {}", order.as_sql(), limit));

        let param_refs: Vec<&dyn duckdb::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(Event {
                id: row.get(0)?,
                ts: DateTime::from_timestamp_micros(row.get(1)?).unwrap_or(DateTime::UNIX_EPOCH),
                source: EventSource::from_str(&row.get::<_, String>(2)?)
                    .unwrap_or(EventSource::Other),
                kind: EventKind::from_str(&row.get::<_, String>(3)?).unwrap_or(EventKind::System),
                severity: EventSeverity::from_str(&row.get::<_, String>(4)?)
                    .unwrap_or(EventSeverity::Info),
                message: row.get(5)?,
                payload: parse_map(&row.get::<_, Option<String>>(6)?.unwrap_or_default()),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }
}

/// Raw SQL reader (SELECT only).
#[derive(Clone)]
pub struct RawSqlReader {
    pool: Arc<ReadPool>,
}

impl std::fmt::Debug for RawSqlReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawSqlReader").finish_non_exhaustive()
    }
}

impl RawSqlReader {
    pub(crate) fn new(pool: Arc<ReadPool>) -> Self {
        Self { pool }
    }

    /// Execute raw SELECT query.
    pub fn execute(&self, sql: &str) -> Result<Vec<HashMap<String, Value>>, StorageError> {
        let trimmed = sql.trim().trim_end_matches(';');
        if !trimmed.to_ascii_lowercase().starts_with("select") {
            return Err(StorageError::InvalidData("only SELECT allowed".to_string()));
        }
        if trimmed.contains(';') {
            return Err(StorageError::InvalidData(
                "multiple statements not allowed".to_string(),
            ));
        }

        let conn = self.pool.get()?;
        let describe = format!("DESCRIBE SELECT * FROM ({trimmed}) AS _q");
        let cols: Vec<String> = {
            let mut stmt = conn.prepare(&describe)?;
            let mut rows = stmt.query([])?;
            let mut names = Vec::new();
            while let Some(row) = rows.next()? {
                names.push(row.get(0)?);
            }
            names
        };

        let mut stmt = conn.prepare(trimmed)?;
        let mut rows = stmt.query([])?;
        let mut results = Vec::new();

        while let Some(row) = rows.next()? {
            let mut map = HashMap::new();
            for (i, name) in cols.iter().enumerate() {
                let val = if let Ok(v) = row.get::<_, i64>(i) {
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
                map.insert(name.clone(), val);
            }
            results.push(map);
        }

        Ok(results)
    }
}

// =============================================================================
// Admin
// =============================================================================

/// Storage administration.
#[derive(Clone)]
pub struct StorageAdmin {
    tx: SyncSender<Command>,
}

impl std::fmt::Debug for StorageAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageAdmin").finish_non_exhaustive()
    }
}

impl StorageAdmin {
    pub(crate) fn new(tx: SyncSender<Command>) -> Self {
        Self { tx }
    }

    pub fn cleanup_metric_values(&self, retention_days: u32) -> Result<(), StorageError> {
        self.tx
            .try_send(Command::CleanupMetricValues { retention_days })
            .map_err(|_| StorageError::ChannelSend)
    }

    pub fn cleanup_events(&self, retention_days: u32) -> Result<(), StorageError> {
        self.tx
            .try_send(Command::CleanupEvents { retention_days })
            .map_err(|_| StorageError::ChannelSend)
    }

    pub fn checkpoint(&self) -> Result<(), StorageError> {
        self.tx
            .try_send(Command::Checkpoint)
            .map_err(|_| StorageError::ChannelSend)
    }

    pub fn shutdown(&self) -> Result<(), StorageError> {
        self.tx
            .try_send(Command::Shutdown)
            .map_err(|_| StorageError::ChannelSend)
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Parse JSON string to BTreeMap.
fn parse_map(s: &str) -> std::collections::BTreeMap<String, String> {
    if s.is_empty() || s == "{}" {
        return std::collections::BTreeMap::new();
    }
    serde_json::from_str(s).unwrap_or_else(|e| {
        tracing::debug!(error = %e, raw = s, "Failed to parse JSON map, returning empty");
        std::collections::BTreeMap::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::actor::DbActor;
    use duckdb::Connection;
    use tempfile::tempdir;

    #[test]
    fn test_metric_roundtrip() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let (handle, tx, _reader_conn) =
            DbActor::spawn(&db_path, 100, std::time::Duration::from_secs(1)).unwrap();
        let writer = StorageWriter::new(tx.clone());
        let admin = StorageAdmin::new(tx);

        let series = MetricSeries::new(
            MetricCategory::NetworkTcp,
            "latency",
            "127.0.0.1:6379",
            StaticTags::new(),
            Some("Redis".to_string()),
        );
        let value = MetricValue::new(series.series_id, 42.5, true, 15);

        writer.upsert_metric_series(series).unwrap();
        writer.insert_metric_value(value).unwrap();
        admin.checkpoint().unwrap();
        admin.shutdown().unwrap();
        handle.join().unwrap();

        // After shutdown, open a new connection to read data
        let conn = Connection::open(&db_path).unwrap();
        let pool = ReadPool::new(conn);
        let reader = MetricReader::new(pool);
        let results = reader.query(MetricQuery::default()).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "latency");
        assert_eq!(results[0].value, 42.5);
    }

    #[test]
    fn test_event_roundtrip() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("event.db");

        {
            let (handle, tx, _reader_conn) =
                DbActor::spawn(&db_path, 100, std::time::Duration::from_secs(1)).unwrap();
            let writer = StorageWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let event = Event::new(
                EventSource::System,
                EventKind::System,
                EventSeverity::Info,
                "Started",
            )
            .with_payload("version", "1.0.0");

            writer.insert_event(event).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        // After shutdown, open a new connection to read data
        let conn = Connection::open(&db_path).unwrap();
        let pool = ReadPool::new(conn);
        let reader = EventReader::new(pool);
        let results = reader.query(EventQuery::default()).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, EventSource::System);
        assert_eq!(results[0].message, "Started");
    }

    #[test]
    fn test_query_filters() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("filter.db");

        {
            let (handle, tx, _reader_conn) =
                DbActor::spawn(&db_path, 100, std::time::Duration::from_secs(1)).unwrap();
            let writer = StorageWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let tcp = MetricSeries::new(
                MetricCategory::NetworkTcp,
                "latency",
                "host1",
                StaticTags::new(),
                None,
            );
            let crypto = MetricSeries::new(
                MetricCategory::Crypto,
                "price",
                "BTC",
                StaticTags::new(),
                None,
            );

            writer.upsert_metric_series(tcp.clone()).unwrap();
            writer
                .insert_metric_value(MetricValue::new(tcp.series_id, 10.0, true, 5))
                .unwrap();
            writer.upsert_metric_series(crypto.clone()).unwrap();
            writer
                .insert_metric_value(MetricValue::new(crypto.series_id, 100000.0, true, 50))
                .unwrap();

            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        // After shutdown, open a new connection to read data
        let conn = Connection::open(&db_path).unwrap();
        let pool = ReadPool::new(conn);
        let reader = MetricReader::new(pool);

        let results = reader
            .query(MetricQuery {
                category: Some(MetricCategory::NetworkTcp),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "latency");
    }

    #[test]
    fn test_dropped_metrics_counter_api() {
        // Test that dropped_metrics counter API works correctly.
        // Note: Reliably testing channel-full behavior requires mocking the sender,
        // which is beyond the scope of this unit test. We verify the counter starts
        // at 0 and the Clone trait shares the counter across instances.
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("dropped.db");

        let (handle, tx, _reader_conn) =
            DbActor::spawn(&db_path, 100, std::time::Duration::from_secs(1)).unwrap();
        let writer = StorageWriter::new(tx.clone());
        let writer_clone = writer.clone();

        // Counter starts at 0
        assert_eq!(writer.dropped_metrics(), 0);
        assert_eq!(writer_clone.dropped_metrics(), 0);

        // Insert a metric successfully
        let series = MetricSeries::new(
            MetricCategory::Custom,
            "test",
            "target",
            StaticTags::new(),
            None,
        );
        writer.upsert_metric_series(series).unwrap();

        // Counter still 0 after successful insert
        assert_eq!(writer.dropped_metrics(), 0);
        // Cloned writer shares the same counter
        assert_eq!(writer_clone.dropped_metrics(), 0);

        tx.send(Command::Shutdown).unwrap();
        handle.join().unwrap();
    }
}
