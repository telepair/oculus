//! Writer actor with dedicated connection and MPSC channel.
//!
//! Single-writer pattern: one thread owns write connection, processes commands via MPSC.
//! Implements batch buffering: flushes when buffer reaches 500 items or 1 second elapsed.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::Utc;
use duckdb::Connection;

use crate::storage::StorageError;
use crate::storage::schema::init_schema;
use crate::storage::types::{Event, MetricSeries, MetricValue};

// =============================================================================
// Constants
// =============================================================================

/// Maximum items in buffer before flush.
const BATCH_SIZE_THRESHOLD: usize = 500;

/// Maximum time before buffer flush.
const BATCH_TIME_THRESHOLD: Duration = Duration::from_secs(1);

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
    /// Cleanup metric values older than retention_days (immediate cleanup).
    CleanupMetricValues { retention_days: u32 },
    /// Cleanup events older than retention_days (immediate cleanup).
    CleanupEvents { retention_days: u32 },
    /// Force flush all buffers.
    Flush,
    /// Force WAL checkpoint.
    Checkpoint,
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
}

impl<T> BatchBuffer<T> {
    fn new() -> Self {
        Self {
            items: Vec::with_capacity(BATCH_SIZE_THRESHOLD),
            last_flush: Instant::now(),
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
        self.items.len() >= BATCH_SIZE_THRESHOLD
            || (!self.items.is_empty() && self.last_flush.elapsed() >= BATCH_TIME_THRESHOLD)
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
    conn: Connection,
    rx: Receiver<Command>,
    value_buffer: BatchBuffer<MetricValue>,
    last_checkpoint: Instant,
    checkpoint_interval: Duration,
}

impl DbActor {
    /// Spawn the writer actor thread.
    ///
    /// Returns a tuple of:
    /// - `JoinHandle<()>`: Handle to the actor thread
    /// - `SyncSender<Command>`: Channel sender for commands
    /// - `Connection`: A cloneable connection for creating reader connections via `try_clone()`
    pub fn spawn(
        db_path: &Path,
        channel_capacity: usize,
        checkpoint_interval: Duration,
    ) -> Result<(JoinHandle<()>, SyncSender<Command>, Connection), StorageError> {
        let (tx, rx) = mpsc::sync_channel(channel_capacity);
        let conn = Connection::open(db_path)?;
        init_schema(&conn)?;

        // Create a cloneable connection for readers before moving conn to actor.
        // DuckDB connections from try_clone() share the same underlying database instance,
        // enabling readers to see writes without waiting for WAL checkpoint.
        let reader_conn = conn.try_clone()?;

        let mut actor = DbActor {
            conn,
            rx,
            value_buffer: BatchBuffer::new(),
            last_checkpoint: Instant::now(),
            checkpoint_interval,
        };
        let handle = thread::spawn(move || actor.run());

        Ok((handle, tx, reader_conn))
    }

    fn run(&mut self) {
        tracing::info!("DbActor started");

        loop {
            let now = Instant::now();
            let flush_deadline = if !self.value_buffer.is_empty() {
                self.value_buffer.last_flush + BATCH_TIME_THRESHOLD
            } else {
                now + Duration::from_secs(60)
            };
            let checkpoint_deadline = self.last_checkpoint + self.checkpoint_interval;

            let deadline = std::cmp::min(flush_deadline, checkpoint_deadline);
            let timeout = deadline.saturating_duration_since(now);

            match self.rx.recv_timeout(timeout) {
                Ok(cmd) => {
                    if self.handle_command(cmd) {
                        break; // Shutdown requested
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Timeout: flush or checkpoint overdue
                }
                Err(RecvTimeoutError::Disconnected) => {
                    tracing::warn!("Channel disconnected, shutting down");
                    self.flush_all();
                    break;
                }
            }

            // Check if any buffer needs flushing
            if self.value_buffer.should_flush() {
                self.flush_all();
            }

            // Check if checkpoint is needed
            if self.last_checkpoint.elapsed() >= self.checkpoint_interval {
                self.flush_all(); // Ensure everything is written before checkpoint
                if let Err(e) = self.checkpoint() {
                    tracing::error!(error = %e, "Periodic checkpoint failed");
                }
                self.last_checkpoint = Instant::now();
            }
        }

        tracing::info!("DbActor stopped");
    }

    fn handle_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::UpsertMetricSeries(series) => {
                // Series: upsert immediately (low volume)
                if let Err(e) = self.upsert_series(&series) {
                    tracing::error!(error = %e, "Series upsert failed");
                }
            }
            Command::InsertMetricValue(value) => {
                // Value: buffer for batch insert (high volume)
                self.value_buffer.push(value);
            }
            Command::InsertEvent(event) => {
                // Event: insert immediately (low volume)
                if let Err(e) = self.insert_event(&event) {
                    tracing::error!(error = %e, "Event insert failed");
                }
            }
            Command::CleanupMetricValues { retention_days } => {
                if let Err(e) = self.cleanup_metric_values(retention_days) {
                    tracing::error!(error = %e, "Cleanup metric values failed");
                }
            }
            Command::CleanupEvents { retention_days } => {
                if let Err(e) = self.cleanup_events(retention_days) {
                    tracing::error!(error = %e, "Cleanup events failed");
                }
            }
            Command::Flush => {
                self.flush_all();
            }
            Command::Checkpoint => {
                self.flush_all();
                if let Err(e) = self.checkpoint() {
                    tracing::error!(error = %e, "Checkpoint failed");
                }
            }
            Command::Shutdown => {
                tracing::info!("DbActor shutting down");
                self.flush_all();
                let _ = self.checkpoint();
                return true;
            }
        }
        false
    }

    fn flush_all(&mut self) {
        if !self.value_buffer.is_empty() {
            let values = self.value_buffer.take();
            if let Err(e) = self.insert_values_batch(&values) {
                tracing::error!(error = %e, count = values.len(), "Values batch insert failed");
            }
        }
    }

    // =========================================================================
    // Insert Operations
    // =========================================================================

    /// Upsert a single metric series (low volume, immediate write).
    fn upsert_series(&self, s: &MetricSeries) -> Result<(), StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO metric_series (series_id, category, name, target, static_tags, description, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (series_id) DO UPDATE SET updated_at = EXCLUDED.updated_at, description = EXCLUDED.description",
        )?;

        let tags_json = serde_json::to_string(&s.static_tags).unwrap_or_else(|_| "{}".to_string());
        stmt.execute(duckdb::params![
            s.series_id,
            s.category.as_ref(),
            &s.name,
            &s.target,
            tags_json,
            s.description.as_deref(),
            s.created_at.timestamp_micros(),
            s.updated_at.timestamp_micros(),
        ])?;

        Ok(())
    }

    /// Batch insert metric values using DuckDB Appender (high volume).
    fn insert_values_batch(&mut self, values: &[MetricValue]) -> Result<(), StorageError> {
        if values.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction()?;
        {
            let mut appender = tx.appender("metric_values")?;

            for v in values {
                let tags_json =
                    serde_json::to_string(&v.dynamic_tags).unwrap_or_else(|_| "{}".to_string());
                appender.append_row(duckdb::params![
                    v.ts.timestamp_micros(),
                    v.series_id,
                    v.value,
                    v.unit.as_deref(),
                    v.success,
                    v.duration_ms,
                    tags_json,
                ])?;
            }
            appender.flush()?;
        }
        tx.commit()?;

        tracing::debug!(count = values.len(), "Values batch inserted");
        Ok(())
    }

    /// Insert a single event (low volume, immediate write).
    fn insert_event(&self, e: &Event) -> Result<(), StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO events (ts, source, kind, severity, message, payload)
             VALUES (?, ?, ?, ?, ?, ?)",
        )?;

        let payload_json = serde_json::to_string(&e.payload).unwrap_or_else(|_| "{}".to_string());
        stmt.execute(duckdb::params![
            e.ts.timestamp_micros(),
            e.source.as_ref(),
            e.kind.as_ref(),
            e.severity.as_ref(),
            &e.message,
            payload_json,
        ])?;

        Ok(())
    }

    // =========================================================================
    // Maintenance Operations
    // =========================================================================

    fn cleanup_metric_values(&self, retention_days: u32) -> Result<(), StorageError> {
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
        let deleted = self.conn.execute(
            "DELETE FROM metric_values WHERE ts < ?",
            [cutoff.timestamp_micros()],
        )?;
        tracing::info!(deleted, retention_days, "Metric values cleaned up");
        Ok(())
    }

    fn cleanup_events(&self, retention_days: u32) -> Result<(), StorageError> {
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
        let deleted = self.conn.execute(
            "DELETE FROM events WHERE ts < ?",
            [cutoff.timestamp_micros()],
        )?;
        tracing::info!(deleted, retention_days, "Events cleaned up");
        Ok(())
    }

    fn checkpoint(&self) -> Result<(), StorageError> {
        self.conn.execute_batch("CHECKPOINT;")?;
        tracing::debug!("WAL checkpoint completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::*;
    use tempfile::tempdir;

    #[test]
    fn test_actor_lifecycle() {
        let dir = tempdir().unwrap();
        let (handle, tx, _reader_conn) =
            DbActor::spawn(&dir.path().join("test.db"), 100, Duration::from_secs(1)).unwrap();
        tx.send(Command::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn test_insert_metric_with_flush() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("metric.db");
        let (handle, tx, _reader_conn) =
            DbActor::spawn(&db_path, 100, Duration::from_secs(1)).unwrap();

        let series = MetricSeries::new(
            MetricCategory::NetworkTcp,
            "latency",
            "127.0.0.1:6379",
            StaticTags::new(),
            Some("Redis".to_string()),
        );
        let value = MetricValue::new(series.series_id, 42.5, true).with_duration_ms(15);

        tx.send(Command::UpsertMetricSeries(series)).unwrap();
        tx.send(Command::InsertMetricValue(value)).unwrap();
        tx.send(Command::Flush).unwrap(); // Force flush
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();
        handle.join().unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metric_values", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_series_deduplication() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("dedup.db");
        let (handle, tx, _reader_conn) =
            DbActor::spawn(&db_path, 100, Duration::from_secs(1)).unwrap();

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
            .unwrap();
        tx.send(Command::InsertMetricValue(MetricValue::new(
            series1.series_id,
            1.0,
            true,
        )))
        .unwrap();
        tx.send(Command::UpsertMetricSeries(series2.clone()))
            .unwrap();
        tx.send(Command::InsertMetricValue(MetricValue::new(
            series2.series_id,
            2.0,
            true,
        )))
        .unwrap();
        tx.send(Command::Flush).unwrap();
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();
        handle.join().unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metric_series", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let desc: String = conn
            .query_row("SELECT description FROM metric_series", [], |r| r.get(0))
            .unwrap();
        assert_eq!(desc, "Second");
    }

    #[test]
    fn test_insert_event_with_flush() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("event.db");
        let (handle, tx, _reader_conn) =
            DbActor::spawn(&db_path, 100, Duration::from_secs(1)).unwrap();

        let event = Event::new(
            EventSource::System,
            EventKind::System,
            EventSeverity::Info,
            "Started",
        )
        .with_payload("version", "1.0.0");

        tx.send(Command::InsertEvent(event)).unwrap();
        tx.send(Command::Flush).unwrap();
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();
        handle.join().unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_batch_threshold() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("batch.db");
        let (handle, tx, _reader_conn) =
            DbActor::spawn(&db_path, 1000, Duration::from_secs(1)).unwrap();

        // Insert exactly BATCH_SIZE_THRESHOLD items
        for i in 0..BATCH_SIZE_THRESHOLD {
            let series = MetricSeries::new(
                MetricCategory::Custom,
                "batch",
                format!("target-{i}"),
                StaticTags::new(),
                None,
            );
            let value = MetricValue::new(series.series_id, i as f64, true);
            tx.send(Command::UpsertMetricSeries(series)).unwrap();
            tx.send(Command::InsertMetricValue(value)).unwrap();
        }

        // Wait for auto-flush (buffer should be full)
        std::thread::sleep(Duration::from_millis(100));
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();
        handle.join().unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metric_values", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, BATCH_SIZE_THRESHOLD as i64);
    }

    #[test]
    fn test_time_based_flush() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("time_flush.db");
        let (handle, tx, _reader_conn) =
            DbActor::spawn(&db_path, 100, Duration::from_secs(1)).unwrap();

        // Insert fewer items than BATCH_SIZE_THRESHOLD
        let series = MetricSeries::new(
            MetricCategory::Custom,
            "time_test",
            "target",
            StaticTags::new(),
            None,
        );
        let value = MetricValue::new(series.series_id, 42.0, true).with_duration_ms(10);

        tx.send(Command::UpsertMetricSeries(series)).unwrap();
        tx.send(Command::InsertMetricValue(value)).unwrap();

        // Wait for time-based flush (BATCH_TIME_THRESHOLD = 1 second)
        std::thread::sleep(Duration::from_millis(1200));

        // Checkpoint and shutdown
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();
        handle.join().unwrap();

        // Verify data was flushed by time threshold
        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metric_values", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "Time-based flush should have written the value");
    }

    #[test]
    fn test_cleanup_operations() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cleanup.db");
        let (handle, tx, _reader_conn) =
            DbActor::spawn(&db_path, 100, Duration::from_secs(1)).unwrap();

        // Insert test data
        let series = MetricSeries::new(
            MetricCategory::Custom,
            "cleanup_test",
            "target",
            StaticTags::new(),
            None,
        );

        // Insert metric value with current timestamp
        let value = MetricValue::new(series.series_id, 100.0, true).with_duration_ms(5);
        tx.send(Command::UpsertMetricSeries(series)).unwrap();
        tx.send(Command::InsertMetricValue(value)).unwrap();

        // Insert event
        let event = Event::new(
            EventSource::System,
            EventKind::System,
            EventSeverity::Info,
            "Test event",
        );
        tx.send(Command::InsertEvent(event)).unwrap();
        tx.send(Command::Flush).unwrap();
        tx.send(Command::Checkpoint).unwrap();

        // Cleanup with 0 retention days (should delete all data)
        // Note: This tests that the cleanup commands execute without error
        tx.send(Command::CleanupMetricValues { retention_days: 0 })
            .unwrap();
        tx.send(Command::CleanupEvents { retention_days: 0 })
            .unwrap();

        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();
        handle.join().unwrap();

        // Verify data was cleaned up
        let conn = Connection::open(&db_path).unwrap();
        let metric_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metric_values", [], |r| r.get(0))
            .unwrap();
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();

        assert_eq!(metric_count, 0, "Cleanup should have deleted metric values");
        assert_eq!(event_count, 0, "Cleanup should have deleted events");
    }
}
