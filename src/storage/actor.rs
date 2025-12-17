//! Writer actor with async task and channel.
//!
//! Single-writer pattern: one task owns write operations, processes commands via async channel.
//! Implements batch buffering: flushes when buffer reaches threshold or time elapsed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use sqlx::SqlitePool;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;

use crate::storage::StorageError;
use crate::storage::db::SqlitePool as DbPool;
use crate::storage::schema::init_schema;
use crate::storage::types::{Event, MetricSeries, MetricValue};

// =============================================================================
// Constants
// =============================================================================

/// Default maximum items in buffer before flush.
pub const DEFAULT_BATCH_SIZE: usize = 500;

/// Default maximum time before buffer flush.
pub const DEFAULT_BATCH_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// Default channel capacity.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 10_000;

// =============================================================================
// Commands
// =============================================================================

/// Commands sent to the writer actor.
#[derive(Debug)]
pub enum Command {
    /// Upsert metric series.
    UpsertMetricSeries(MetricSeries),
    /// Insert metric value (batch insert).
    InsertMetricValue(MetricValue),
    /// Insert event (immediate insert).
    InsertEvent(Event),
    /// Cleanup metric values older than retention_days.
    CleanupMetricValues { retention_days: u32 },
    /// Cleanup events older than retention_days.
    CleanupEvents { retention_days: u32 },
    /// Force flush all buffers.
    Flush,
    /// Graceful shutdown.
    Shutdown,
}

// =============================================================================
// Buffers
// =============================================================================

/// Buffer for batch inserts with time-based and size-based flushing.
struct BatchBuffer<T> {
    items: Vec<T>,
    last_flush: Instant,
    size_threshold: usize,
    time_threshold: Duration,
}

impl<T> BatchBuffer<T> {
    fn new(size_threshold: usize, time_threshold: Duration) -> Self {
        Self {
            items: Vec::with_capacity(size_threshold),
            last_flush: Instant::now(),
            size_threshold,
            time_threshold,
        }
    }

    fn push(&mut self, item: T) {
        // Reset flush timer on first item to avoid treating long-idle buffers as overdue
        if self.items.is_empty() {
            self.last_flush = Instant::now();
        }
        self.items.push(item);
    }

    fn should_flush(&self) -> bool {
        self.items.len() >= self.size_threshold
            || (!self.items.is_empty() && self.last_flush.elapsed() >= self.time_threshold)
    }

    fn time_until_flush(&self) -> Duration {
        if self.items.is_empty() {
            Duration::from_secs(60)
        } else {
            self.time_threshold
                .saturating_sub(self.last_flush.elapsed())
        }
    }

    fn take(&mut self) -> Vec<T> {
        self.last_flush = Instant::now();
        std::mem::take(&mut self.items)
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

// =============================================================================
// Actor
// =============================================================================

/// Database writer actor with batch buffering for metric values.
///
/// Only `MetricValue` uses batch buffering for high-throughput insertion.
/// `MetricSeries` and `Event` are inserted immediately due to lower volume.
pub struct DbActor {
    pool: SqlitePool,
    rx: Receiver<Command>,
    value_buffer: BatchBuffer<MetricValue>,
}

impl DbActor {
    /// Spawn the writer actor task.
    ///
    /// Returns a tuple of:
    /// - `JoinHandle<()>`: Handle to the actor task
    /// - `Sender<Command>`: Channel sender for commands
    /// - `Arc<DbPool>`: Shared pool for reader connections
    ///
    /// # Parameters
    /// - `db_url` - SQLite connection URL, e.g., `sqlite:data/oculus.db?mode=rwc`
    /// - `channel_capacity` - Maximum number of commands queued
    /// - `batch_size` - Maximum items in buffer before flush
    /// - `batch_flush_interval` - Maximum time before buffer flush
    pub async fn spawn(
        db_url: &str,
        channel_capacity: usize,
        batch_size: usize,
        batch_flush_interval: Duration,
    ) -> Result<(JoinHandle<()>, Sender<Command>, Arc<DbPool>), StorageError> {
        let (tx, rx) = mpsc::channel(channel_capacity);

        let db_pool = DbPool::connect(db_url).await?;
        init_schema(db_pool.inner()).await?;

        let pool = db_pool.inner().clone();
        let shared_pool = Arc::new(db_pool);

        let mut actor = DbActor {
            pool,
            rx,
            value_buffer: BatchBuffer::new(batch_size, batch_flush_interval),
        };

        let handle = tokio::spawn(async move { actor.run().await });

        Ok((handle, tx, shared_pool))
    }

    async fn run(&mut self) {
        tracing::info!("DbActor started");

        loop {
            let timeout = self.value_buffer.time_until_flush();

            tokio::select! {
                Some(cmd) = self.rx.recv() => {
                    if self.handle_command(cmd).await {
                        break; // Shutdown requested
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    // Timeout: flush if buffer has items
                    if self.value_buffer.should_flush() {
                        self.flush_all().await;
                    }
                }
            }

            // Check if buffer needs flushing after command
            if self.value_buffer.should_flush() {
                self.flush_all().await;
            }
        }

        tracing::info!("DbActor stopped");
    }

    async fn handle_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::UpsertMetricSeries(series) => {
                if let Err(e) = self.upsert_series(&series).await {
                    tracing::error!(error = %e, "Series upsert failed");
                }
            }
            Command::InsertMetricValue(value) => {
                self.value_buffer.push(value);
            }
            Command::InsertEvent(event) => {
                if let Err(e) = self.insert_event(&event).await {
                    tracing::error!(error = %e, "Event insert failed");
                }
            }
            Command::CleanupMetricValues { retention_days } => {
                if let Err(e) = self.cleanup_metric_values(retention_days).await {
                    tracing::error!(error = %e, "Cleanup metric values failed");
                }
            }
            Command::CleanupEvents { retention_days } => {
                if let Err(e) = self.cleanup_events(retention_days).await {
                    tracing::error!(error = %e, "Cleanup events failed");
                }
            }
            Command::Flush => {
                self.flush_all().await;
            }
            Command::Shutdown => {
                tracing::info!("DbActor shutting down");
                self.flush_all().await;
                return true;
            }
        }
        false
    }

    async fn flush_all(&mut self) {
        if !self.value_buffer.is_empty() {
            let values = self.value_buffer.take();
            if let Err(e) = self.insert_values_batch(&values).await {
                tracing::error!(error = %e, count = values.len(), "Values batch insert failed");
            }
        }
    }

    // =========================================================================
    // Insert Operations
    // =========================================================================

    /// Upsert a single metric series (low volume, immediate write).
    async fn upsert_series(&self, s: &MetricSeries) -> Result<(), StorageError> {
        let tags_json = serde_json::to_string(&s.static_tags).unwrap_or_else(|e| {
            tracing::warn!(error = %e, series_id = s.series_id, "Failed to serialize static_tags, using empty object");
            "{}".to_string()
        });

        sqlx::query(
            "INSERT OR REPLACE INTO metric_series (series_id, category, name, target, static_tags, description, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(s.series_id as i64)
        .bind(s.category.as_ref())
        .bind(&s.name)
        .bind(&s.target)
        .bind(&tags_json)
        .bind(s.description.as_deref())
        .bind(s.created_at.timestamp_micros())
        .bind(s.updated_at.timestamp_micros())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Batch insert metric values using transaction.
    async fn insert_values_batch(&mut self, values: &[MetricValue]) -> Result<(), StorageError> {
        if values.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        for v in values {
            let tags_json = serde_json::to_string(&v.dynamic_tags).unwrap_or_else(|e| {
                tracing::warn!(error = %e, series_id = v.series_id, "Failed to serialize dynamic_tags, using empty object");
                "{}".to_string()
            });

            sqlx::query(
                "INSERT INTO metric_values (ts, series_id, value, unit, success, duration_ms, dynamic_tags)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(v.ts.timestamp_micros())
            .bind(v.series_id as i64)
            .bind(v.value)
            .bind(v.unit.as_deref())
            .bind(v.success)
            .bind(i64::from(v.duration_ms))
            .bind(&tags_json)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        tracing::debug!(count = values.len(), "Values batch inserted");
        Ok(())
    }

    /// Insert a single event (low volume, immediate write).
    async fn insert_event(&self, e: &Event) -> Result<(), StorageError> {
        let payload_json = serde_json::to_string(&e.payload).unwrap_or_else(|err| {
            tracing::warn!(error = %err, message = %e.message, "Failed to serialize event payload, using empty object");
            "{}".to_string()
        });

        sqlx::query(
            "INSERT INTO events (ts, source, kind, severity, message, payload)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(e.ts.timestamp_micros())
        .bind(e.source.as_ref())
        .bind(e.kind.as_ref())
        .bind(e.severity.as_ref())
        .bind(&e.message)
        .bind(&payload_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // =========================================================================
    // Maintenance Operations
    // =========================================================================

    async fn cleanup_metric_values(&self, retention_days: u32) -> Result<(), StorageError> {
        let cutoff = chrono::TimeDelta::try_days(i64::from(retention_days))
            .map(|d| Utc::now() - d)
            .unwrap_or_else(Utc::now);

        let result = sqlx::query("DELETE FROM metric_values WHERE ts < ?")
            .bind(cutoff.timestamp_micros())
            .execute(&self.pool)
            .await?;

        tracing::info!(
            deleted = result.rows_affected(),
            retention_days,
            "Metric values cleaned up"
        );
        Ok(())
    }

    async fn cleanup_events(&self, retention_days: u32) -> Result<(), StorageError> {
        let cutoff = chrono::TimeDelta::try_days(i64::from(retention_days))
            .map(|d| Utc::now() - d)
            .unwrap_or_else(Utc::now);

        let result = sqlx::query("DELETE FROM events WHERE ts < ?")
            .bind(cutoff.timestamp_micros())
            .execute(&self.pool)
            .await?;

        tracing::info!(
            deleted = result.rows_affected(),
            retention_days,
            "Events cleaned up"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::*;

    #[tokio::test]
    async fn test_actor_lifecycle() {
        let (handle, tx, _pool) = DbActor::spawn(
            "sqlite::memory:",
            100,
            DEFAULT_BATCH_SIZE,
            DEFAULT_BATCH_FLUSH_INTERVAL,
        )
        .await
        .unwrap();

        tx.send(Command::Shutdown).await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_insert_metric_with_flush() {
        let (handle, tx, pool) = DbActor::spawn(
            "sqlite::memory:",
            100,
            DEFAULT_BATCH_SIZE,
            DEFAULT_BATCH_FLUSH_INTERVAL,
        )
        .await
        .unwrap();

        let series = MetricSeries::new(
            MetricCategory::NetworkTcp,
            "latency",
            "127.0.0.1:6379",
            StaticTags::new(),
            Some("Redis".to_string()),
        );
        let value = MetricValue::new(series.series_id, 42.5, true).with_duration_ms(15);

        tx.send(Command::UpsertMetricSeries(series)).await.unwrap();
        tx.send(Command::InsertMetricValue(value)).await.unwrap();
        tx.send(Command::Flush).await.unwrap();

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        tx.send(Command::Shutdown).await.unwrap();
        handle.await.unwrap();

        let count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM metric_values")
            .fetch_one(pool.inner())
            .await
            .unwrap();
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn test_series_deduplication() {
        let (handle, tx, pool) = DbActor::spawn(
            "sqlite::memory:",
            100,
            DEFAULT_BATCH_SIZE,
            DEFAULT_BATCH_FLUSH_INTERVAL,
        )
        .await
        .unwrap();

        let series1 = MetricSeries::new(
            MetricCategory::Custom,
            "test",
            "t",
            StaticTags::new(),
            Some("First".to_string()),
        );
        let series2 = MetricSeries::new(
            MetricCategory::Custom,
            "test",
            "t",
            StaticTags::new(),
            Some("Second".to_string()),
        );

        assert_eq!(series1.series_id, series2.series_id);

        tx.send(Command::UpsertMetricSeries(series1.clone()))
            .await
            .unwrap();
        tx.send(Command::InsertMetricValue(MetricValue::new(
            series1.series_id,
            1.0,
            true,
        )))
        .await
        .unwrap();
        tx.send(Command::UpsertMetricSeries(series2.clone()))
            .await
            .unwrap();
        tx.send(Command::InsertMetricValue(MetricValue::new(
            series2.series_id,
            2.0,
            true,
        )))
        .await
        .unwrap();
        tx.send(Command::Flush).await.unwrap();

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        tx.send(Command::Shutdown).await.unwrap();
        handle.await.unwrap();

        let count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM metric_series")
            .fetch_one(pool.inner())
            .await
            .unwrap();
        assert_eq!(count.0, 1);

        let desc: (String,) = sqlx::query_as("SELECT description FROM metric_series")
            .fetch_one(pool.inner())
            .await
            .unwrap();
        assert_eq!(desc.0, "Second");
    }

    #[tokio::test]
    async fn test_insert_event() {
        let (handle, tx, pool) = DbActor::spawn(
            "sqlite::memory:",
            100,
            DEFAULT_BATCH_SIZE,
            DEFAULT_BATCH_FLUSH_INTERVAL,
        )
        .await
        .unwrap();

        let event = Event::new(
            EventSource::System,
            EventKind::System,
            EventSeverity::Info,
            "Started",
        )
        .with_payload("version", "1.0.0");

        tx.send(Command::InsertEvent(event)).await.unwrap();

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        tx.send(Command::Shutdown).await.unwrap();
        handle.await.unwrap();

        let count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM events")
            .fetch_one(pool.inner())
            .await
            .unwrap();
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn test_cleanup_operations() {
        let (handle, tx, pool) = DbActor::spawn(
            "sqlite::memory:",
            100,
            DEFAULT_BATCH_SIZE,
            DEFAULT_BATCH_FLUSH_INTERVAL,
        )
        .await
        .unwrap();

        // Insert test data
        let series = MetricSeries::new(
            MetricCategory::Custom,
            "cleanup_test",
            "target",
            StaticTags::new(),
            None,
        );
        let value = MetricValue::new(series.series_id, 100.0, true).with_duration_ms(5);

        tx.send(Command::UpsertMetricSeries(series)).await.unwrap();
        tx.send(Command::InsertMetricValue(value)).await.unwrap();

        let event = Event::new(
            EventSource::System,
            EventKind::System,
            EventSeverity::Info,
            "Test event",
        );
        tx.send(Command::InsertEvent(event)).await.unwrap();
        tx.send(Command::Flush).await.unwrap();

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Cleanup with 0 retention days (should delete all data)
        tx.send(Command::CleanupMetricValues { retention_days: 0 })
            .await
            .unwrap();
        tx.send(Command::CleanupEvents { retention_days: 0 })
            .await
            .unwrap();

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        tx.send(Command::Shutdown).await.unwrap();
        handle.await.unwrap();

        let metric_count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM metric_values")
            .fetch_one(pool.inner())
            .await
            .unwrap();
        let event_count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM events")
            .fetch_one(pool.inner())
            .await
            .unwrap();

        assert_eq!(
            metric_count.0, 0,
            "Cleanup should have deleted metric values"
        );
        assert_eq!(event_count.0, 0, "Cleanup should have deleted events");
    }
}
