//! User-facing storage facades.
//!
//! Provides ergonomic APIs for storage operations:
//! - `StorageWriter`: Non-blocking writes via async channel
//! - `MetricReader`: Query metrics (series + values JOIN)
//! - `EventReader`: Query events
//! - `StorageAdmin`: Cleanup and maintenance

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::Row;
use strum_macros::{AsRefStr, EnumString};
use tokio::sync::mpsc::Sender;

use crate::storage::StorageError;
use crate::storage::actor::Command;
use crate::storage::db::SqlitePool;
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
    pub unit: Option<String>,
    pub success: bool,
    pub duration_ms: u32,
    pub dynamic_tags: DynamicTags,
}

/// Statistics for a single category.
#[derive(Debug, Clone, Serialize)]
pub struct CategoryStats {
    pub category: String,
    pub total: u64,
    pub success: u64,
    pub failure: u64,
}

/// Aggregated metric statistics grouped by category and success.
#[derive(Debug, Clone, Serialize)]
pub struct MetricStats {
    pub total: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub by_category: Vec<CategoryStats>,
}

// =============================================================================
// Writer
// =============================================================================

/// Non-blocking storage writer.
///
/// Uses `try_send` - data is dropped if channel is full.
#[derive(Clone)]
pub struct StorageWriter {
    tx: Sender<Command>,
    dropped_metrics: Arc<AtomicU64>,
}

impl std::fmt::Debug for StorageWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageWriter").finish_non_exhaustive()
    }
}

impl StorageWriter {
    pub(crate) fn new(tx: Sender<Command>) -> Self {
        Self {
            tx,
            dropped_metrics: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get total count of dropped metrics due to channel capacity.
    pub fn dropped_metrics(&self) -> u64 {
        self.dropped_metrics.load(Ordering::Relaxed)
    }

    /// Upsert a metric series.
    pub fn upsert_metric_series(&self, series: MetricSeries) -> Result<(), StorageError> {
        if self
            .tx
            .try_send(Command::UpsertMetricSeries(series))
            .is_err()
        {
            self.dropped_metrics.fetch_add(1, Ordering::Relaxed);
            return Err(StorageError::ChannelSend);
        }
        Ok(())
    }

    /// Insert a metric value.
    pub fn insert_metric_value(&self, value: MetricValue) -> Result<(), StorageError> {
        if self.tx.try_send(Command::InsertMetricValue(value)).is_err() {
            tracing::warn!("Channel full, dropping metric value");
            self.dropped_metrics.fetch_add(1, Ordering::Relaxed);
            return Err(StorageError::ChannelSend);
        }
        Ok(())
    }

    /// Insert a single event.
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
    pool: Arc<SqlitePool>,
}

impl std::fmt::Debug for MetricReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricReader").finish_non_exhaustive()
    }
}

impl MetricReader {
    pub(crate) fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    /// Query metrics with filters.
    pub async fn query(&self, q: MetricQuery) -> Result<Vec<MetricResult>, StorageError> {
        let now = Utc::now();
        let start = q
            .start
            .unwrap_or_else(|| now - Duration::days(DEFAULT_RANGE_DAYS));
        let end = q.end.unwrap_or(now);
        let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let order = q.order.unwrap_or_default();

        // Build dynamic query
        let mut sql = String::from(
            "SELECT s.series_id, s.category, s.name, s.target, s.static_tags, s.description,
                    v.ts, v.value, v.unit, v.success, v.duration_ms, v.dynamic_tags
             FROM metric_values v
             JOIN metric_series s ON v.series_id = s.series_id
             WHERE v.ts >= ? AND v.ts <= ?",
        );

        if q.category.is_some() {
            sql.push_str(" AND s.category = ?");
        }
        if q.name.is_some() {
            sql.push_str(" AND s.name = ?");
        }
        if q.target.is_some() {
            sql.push_str(" AND s.target = ?");
        }

        // Note: ORDER BY direction must be embedded (ASC/DESC can't be parameterized),
        // but LIMIT can be safely bound as a parameter
        sql.push_str(&format!(" ORDER BY v.ts {} LIMIT ?", order.as_sql()));

        // Build query with bindings
        let mut query = sqlx::query(&sql)
            .bind(start.timestamp_micros())
            .bind(end.timestamp_micros());

        if let Some(cat) = &q.category {
            query = query.bind(cat.as_ref());
        }
        if let Some(name) = &q.name {
            query = query.bind(name);
        }
        if let Some(target) = &q.target {
            query = query.bind(target);
        }

        // LIMIT must be bound last to match placeholder order in SQL
        query = query.bind(limit as i64);

        let rows = query.fetch_all(self.pool.inner()).await?;

        let results: Vec<MetricResult> = rows
            .into_iter()
            .map(|row| MetricResult {
                series_id: row.get::<i64, _>(0) as u64,
                category: MetricCategory::from_str(row.get::<&str, _>(1))
                    .unwrap_or(MetricCategory::Custom),
                name: row.get(2),
                target: row.get(3),
                static_tags: parse_map(row.get::<Option<&str>, _>(4).unwrap_or("{}")),
                description: row.get(5),
                ts: DateTime::from_timestamp_micros(row.get(6)).unwrap_or(DateTime::UNIX_EPOCH),
                value: row.get(7),
                unit: row.get(8),
                success: row.get::<i32, _>(9) != 0,
                duration_ms: row.get::<Option<i64>, _>(10).unwrap_or(0) as u32,
                dynamic_tags: parse_map(row.get::<Option<&str>, _>(11).unwrap_or("{}")),
            })
            .collect();

        Ok(results)
    }

    /// Get aggregated statistics grouped by category and success.
    pub async fn stats(
        &self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<MetricStats, StorageError> {
        let now = Utc::now();
        let start_ts = start.unwrap_or_else(|| now - Duration::days(DEFAULT_RANGE_DAYS));
        let end_ts = end.unwrap_or(now);

        let sql = "SELECT s.category, v.success, COUNT(*) as cnt
                   FROM metric_values v
                   JOIN metric_series s ON v.series_id = s.series_id
                   WHERE v.ts >= ? AND v.ts <= ?
                   GROUP BY s.category, v.success
                   ORDER BY s.category";

        let rows = sqlx::query(sql)
            .bind(start_ts.timestamp_micros())
            .bind(end_ts.timestamp_micros())
            .fetch_all(self.pool.inner())
            .await?;

        // Aggregate into CategoryStats
        let mut category_map: HashMap<String, CategoryStats> = HashMap::new();
        let mut total = 0u64;
        let mut success_count = 0u64;
        let mut failure_count = 0u64;

        for row in rows {
            let cat: String = row.get(0);
            let success: i32 = row.get(1);
            let cnt: i64 = row.get(2);
            let cnt = cnt as u64;

            total += cnt;
            if success != 0 {
                success_count += cnt;
            } else {
                failure_count += cnt;
            }

            let entry = category_map.entry(cat.clone()).or_insert(CategoryStats {
                category: cat,
                total: 0,
                success: 0,
                failure: 0,
            });
            entry.total += cnt;
            if success != 0 {
                entry.success += cnt;
            } else {
                entry.failure += cnt;
            }
        }

        let mut by_category: Vec<CategoryStats> = category_map.into_values().collect();
        by_category.sort_by(|a, b| a.category.cmp(&b.category));

        Ok(MetricStats {
            total,
            success_count,
            failure_count,
            by_category,
        })
    }
}

/// Event reader.
#[derive(Clone)]
pub struct EventReader {
    pool: Arc<SqlitePool>,
}

impl std::fmt::Debug for EventReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventReader").finish_non_exhaustive()
    }
}

impl EventReader {
    pub(crate) fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    /// Query events with filters.
    pub async fn query(&self, q: EventQuery) -> Result<Vec<Event>, StorageError> {
        let now = Utc::now();
        let start = q
            .start
            .unwrap_or_else(|| now - Duration::days(DEFAULT_RANGE_DAYS));
        let end = q.end.unwrap_or(now);
        let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let order = q.order.unwrap_or_default();

        let mut sql = String::from(
            "SELECT id, ts, source, kind, severity, message, payload
             FROM events WHERE ts >= ? AND ts <= ?",
        );

        if q.source.is_some() {
            sql.push_str(" AND source = ?");
        }
        if q.kind.is_some() {
            sql.push_str(" AND kind = ?");
        }
        if q.severity.is_some() {
            sql.push_str(" AND severity = ?");
        }

        // Note: ORDER BY direction must be embedded, but LIMIT is parameterized
        sql.push_str(&format!(" ORDER BY ts {} LIMIT ?", order.as_sql()));

        let mut query = sqlx::query(&sql)
            .bind(start.timestamp_micros())
            .bind(end.timestamp_micros());

        if let Some(src) = &q.source {
            query = query.bind(src.as_ref());
        }
        if let Some(kind) = &q.kind {
            query = query.bind(kind.as_ref());
        }
        if let Some(sev) = &q.severity {
            query = query.bind(sev.as_ref());
        }

        // LIMIT must be bound last to match placeholder order in SQL
        query = query.bind(limit as i64);

        let rows = query.fetch_all(self.pool.inner()).await?;

        let results: Vec<Event> = rows
            .into_iter()
            .map(|row| Event {
                id: Some(row.get::<i64, _>(0)),
                ts: DateTime::from_timestamp_micros(row.get(1)).unwrap_or(DateTime::UNIX_EPOCH),
                source: EventSource::from_str(row.get::<&str, _>(2)).unwrap_or(EventSource::System),
                kind: EventKind::from_str(row.get::<&str, _>(3)).unwrap_or(EventKind::System),
                severity: EventSeverity::from_str(row.get::<&str, _>(4))
                    .unwrap_or(EventSeverity::Info),
                message: row.get(5),
                payload: parse_map(row.get::<Option<&str>, _>(6).unwrap_or("{}")),
            })
            .collect();

        Ok(results)
    }
}

// =============================================================================
// Admin
// =============================================================================

/// Storage administration.
#[derive(Clone)]
pub struct StorageAdmin {
    tx: Sender<Command>,
}

impl std::fmt::Debug for StorageAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageAdmin").finish_non_exhaustive()
    }
}

impl StorageAdmin {
    pub(crate) fn new(tx: Sender<Command>) -> Self {
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
    use std::time::Duration;

    #[tokio::test]
    async fn test_metric_roundtrip() {
        let (handle, tx, pool) = DbActor::spawn(
            "sqlite::memory:",
            100,
            crate::storage::actor::DEFAULT_BATCH_SIZE,
            crate::storage::actor::DEFAULT_BATCH_FLUSH_INTERVAL,
        )
        .await
        .unwrap();

        let writer = StorageWriter::new(tx.clone());
        let admin = StorageAdmin::new(tx);

        let series = MetricSeries::new(
            MetricCategory::NetworkTcp,
            "latency",
            "127.0.0.1:6379",
            StaticTags::new(),
            Some("Redis".to_string()),
        );
        let value = MetricValue::new(series.series_id, 42.5, true).with_duration_ms(15);

        writer.upsert_metric_series(series).unwrap();
        writer.insert_metric_value(value).unwrap();
        writer.flush().unwrap();

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        admin.shutdown().unwrap();
        handle.await.unwrap();

        let reader = MetricReader::new(pool);
        let results = reader.query(MetricQuery::default()).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "latency");
        assert_eq!(results[0].value, 42.5);
    }

    #[tokio::test]
    async fn test_event_roundtrip() {
        let (handle, tx, pool) = DbActor::spawn(
            "sqlite::memory:",
            100,
            crate::storage::actor::DEFAULT_BATCH_SIZE,
            crate::storage::actor::DEFAULT_BATCH_FLUSH_INTERVAL,
        )
        .await
        .unwrap();

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

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        admin.shutdown().unwrap();
        handle.await.unwrap();

        let reader = EventReader::new(pool);
        let results = reader.query(EventQuery::default()).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, EventSource::System);
        assert_eq!(results[0].message, "Started");
    }

    #[tokio::test]
    async fn test_query_filters() {
        let (handle, tx, pool) = DbActor::spawn(
            "sqlite::memory:",
            100,
            crate::storage::actor::DEFAULT_BATCH_SIZE,
            crate::storage::actor::DEFAULT_BATCH_FLUSH_INTERVAL,
        )
        .await
        .unwrap();

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
            .insert_metric_value(MetricValue::new(tcp.series_id, 10.0, true).with_duration_ms(5))
            .unwrap();
        writer.upsert_metric_series(crypto.clone()).unwrap();
        writer
            .insert_metric_value(
                MetricValue::new(crypto.series_id, 100000.0, true).with_duration_ms(50),
            )
            .unwrap();
        writer.flush().unwrap();

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        admin.shutdown().unwrap();
        handle.await.unwrap();

        let reader = MetricReader::new(pool);
        let results = reader
            .query(MetricQuery {
                category: Some(MetricCategory::NetworkTcp),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "latency");
    }
}
