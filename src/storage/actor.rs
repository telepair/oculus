//! Writer actor with dedicated connection and MPSC channel.
//!
//! This module implements the single-writer pattern for DuckDB:
//! one thread owns the write connection and processes commands via MPSC.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};

use chrono::Utc;
use duckdb::Connection;

use crate::storage::StorageError;
use crate::storage::schema::init_schema;
use crate::storage::{Event, Metric};

/// Commands sent to the writer actor.
#[derive(Debug)]
pub enum Command {
    /// Insert a single metric.
    InsertMetric(Metric),
    /// Insert a batch of metrics.
    InsertMetrics(Vec<Metric>),
    /// Insert a single event.
    InsertEvent(Event),
    /// Insert a batch of events.
    InsertEvents(Vec<Event>),
    /// Delete metrics older than retention_days.
    CleanupMetrics { retention_days: u32 },
    /// Delete events older than retention_days.
    CleanupEvents { retention_days: u32 },
    /// Force WAL checkpoint for read visibility.
    Checkpoint,
    /// Graceful shutdown.
    Shutdown,
}

/// Database writer actor owning the exclusive write connection.
pub struct DbActor {
    conn: Connection,
    rx: Receiver<Command>,
}

impl DbActor {
    /// Spawn the writer actor thread.
    ///
    /// Returns the thread handle and the command sender.
    pub fn spawn(
        db_path: &Path,
        channel_capacity: usize,
    ) -> Result<(JoinHandle<()>, SyncSender<Command>), StorageError> {
        let (tx, rx) = mpsc::sync_channel(channel_capacity);

        let conn = Connection::open(db_path)?;
        init_schema(&conn)?;

        let mut actor = DbActor { conn, rx };

        let handle = thread::spawn(move || {
            actor.run();
        });

        Ok((handle, tx))
    }

    /// Main event loop: receive and process commands.
    fn run(&mut self) {
        tracing::info!("DbActor started");

        while let Ok(cmd) = self.rx.recv() {
            match cmd {
                Command::InsertMetric(metric) => {
                    if let Err(e) = self.insert_metrics_batch(&[metric]) {
                        tracing::error!("Failed to insert metric");
                        tracing::debug!("Metric insertion error: {e}");
                    }
                }
                Command::InsertMetrics(metrics) => {
                    if let Err(e) = self.insert_metrics_batch(&metrics) {
                        tracing::error!("Failed to insert metrics batch");
                        tracing::debug!("Metrics batch insertion error: {e}");
                    }
                }
                Command::InsertEvent(event) => {
                    if let Err(e) = self.insert_events_batch(&[event]) {
                        tracing::error!("Failed to insert event");
                        tracing::debug!("Event insertion error: {e}");
                    }
                }
                Command::InsertEvents(events) => {
                    if let Err(e) = self.insert_events_batch(&events) {
                        tracing::error!("Failed to insert events batch");
                        tracing::debug!("Events batch insertion error: {e}");
                    }
                }
                Command::CleanupMetrics { retention_days } => {
                    if let Err(e) = self.cleanup_metrics(retention_days) {
                        tracing::error!("Failed to cleanup metrics");
                        tracing::debug!("Metrics cleanup error: {e}");
                    }
                }
                Command::CleanupEvents { retention_days } => {
                    if let Err(e) = self.cleanup_events(retention_days) {
                        tracing::error!("Failed to cleanup events");
                        tracing::debug!("Events cleanup error: {e}");
                    }
                }
                Command::Checkpoint => {
                    if let Err(e) = self.checkpoint() {
                        tracing::error!("Failed to checkpoint");
                        tracing::debug!("Checkpoint error: {e}");
                    }
                }
                Command::Shutdown => {
                    tracing::info!("DbActor shutting down");
                    // Final checkpoint before exit
                    let _ = self.checkpoint();
                    break;
                }
            }
        }

        tracing::info!("DbActor stopped");
    }

    /// Insert metrics using Appender for high-throughput.
    fn insert_metrics_batch(&self, metrics: &[Metric]) -> Result<(), StorageError> {
        let mut appender = self.conn.appender("metrics")?;

        for m in metrics {
            let ts_micros = m.ts.timestamp_micros();
            let tags_json = m.tags.as_ref().map(|t| t.to_string());
            appender.append_row(duckdb::params![
                ts_micros,
                &m.category,
                &m.symbol,
                m.value,
                tags_json,
                m.success,
                m.duration_ms,
            ])?;
            tracing::debug!("Inserted metric: {m:?}");
        }

        appender.flush()?;
        tracing::debug!("Flushed metrics");
        Ok(())
    }

    /// Insert events using prepared statement (Appender has issues with ENUM types).
    fn insert_events_batch(&self, events: &[Event]) -> Result<(), StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO events (ts, source, kind, severity, message, payload) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )?;

        for e in events {
            let ts_micros = e.ts.timestamp_micros();
            let payload_json = e.payload.as_ref().map(|p| p.to_string());
            stmt.execute(duckdb::params![
                ts_micros,
                &e.source,
                e.kind.as_ref(),
                e.severity.as_ref(),
                &e.message,
                payload_json,
            ])?;
            tracing::debug!("Inserted event: {e:?}");
        }

        Ok(())
    }

    /// Delete metrics older than retention_days.
    fn cleanup_metrics(&self, retention_days: u32) -> Result<(), StorageError> {
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
        let cutoff_micros = cutoff.timestamp_micros();

        let deleted = self
            .conn
            .execute("DELETE FROM metrics WHERE ts < ?", [cutoff_micros])?;

        tracing::info!("Cleaned up {deleted} metrics older than {retention_days} days");
        Ok(())
    }

    /// Delete events older than retention_days.
    fn cleanup_events(&self, retention_days: u32) -> Result<(), StorageError> {
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
        let cutoff_micros = cutoff.timestamp_micros();

        let deleted = self
            .conn
            .execute("DELETE FROM events WHERE ts < ?", [cutoff_micros])?;

        tracing::info!("Cleaned up {deleted} events older than {retention_days} days");
        Ok(())
    }

    /// Force WAL checkpoint.
    fn checkpoint(&self) -> Result<(), StorageError> {
        self.conn.execute_batch("CHECKPOINT;")?;
        tracing::debug!("WAL checkpoint completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::{EventKind, EventSeverity};
    use tempfile::tempdir;

    #[test]
    fn test_actor_spawn_and_shutdown() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();

        // Send shutdown command
        tx.send(Command::Shutdown).unwrap();

        // Wait for actor to finish
        handle.join().unwrap();
    }

    #[test]
    fn test_actor_insert_metric() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();

        let metric = Metric {
            ts: Utc::now(),
            category: "test".to_string(),
            symbol: "test.metric".to_string(),
            value: 42.0,
            tags: None,
            success: true,
            duration_ms: 0,
        };

        tx.send(Command::InsertMetric(metric)).unwrap();
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();

        handle.join().unwrap();

        // Verify data was written
        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metrics", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_actor_insert_event() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("event_test.db");

        let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();

        let event = Event {
            id: None,
            ts: Utc::now(),
            source: "test.actor".to_string(),
            kind: EventKind::System,
            severity: EventSeverity::Info,
            message: "Actor test event".to_string(),
            payload: Some(serde_json::json!({"test": true})),
        };

        tx.send(Command::InsertEvent(event)).unwrap();
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();

        handle.join().unwrap();

        // Verify event was written
        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Verify event has auto-generated id
        let id: i64 = conn
            .query_row("SELECT id FROM events", [], |row| row.get(0))
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_actor_insert_events_batch() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("events_batch.db");

        let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();

        let events: Vec<Event> = (0..5)
            .map(|i| Event {
                id: None,
                ts: Utc::now(),
                source: format!("batch.source.{i}"),
                kind: EventKind::Alert,
                severity: EventSeverity::Warn,
                message: format!("Batch event {i}"),
                payload: None,
            })
            .collect();

        tx.send(Command::InsertEvents(events)).unwrap();
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();

        handle.join().unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_actor_cleanup_metrics() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cleanup_metrics.db");

        let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();

        // Insert old metric (10 days ago)
        let old_ts = Utc::now() - chrono::Duration::days(10);
        let old_metric = Metric {
            ts: old_ts,
            category: "cleanup".to_string(),
            symbol: "old.metric".to_string(),
            value: 1.0,
            tags: None,
            success: true,
            duration_ms: 0,
        };
        tx.send(Command::InsertMetric(old_metric)).unwrap();

        // Insert recent metric
        let new_metric = Metric {
            ts: Utc::now(),
            category: "cleanup".to_string(),
            symbol: "new.metric".to_string(),
            value: 2.0,
            tags: None,
            success: true,
            duration_ms: 0,
        };
        tx.send(Command::InsertMetric(new_metric)).unwrap();
        tx.send(Command::Checkpoint).unwrap();

        // Cleanup metrics older than 7 days
        tx.send(Command::CleanupMetrics { retention_days: 7 })
            .unwrap();
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();

        handle.join().unwrap();

        // Verify only recent metric remains
        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metrics", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let symbol: String = conn
            .query_row("SELECT symbol FROM metrics", [], |row| row.get(0))
            .unwrap();
        assert_eq!(symbol, "new.metric");
    }

    #[test]
    fn test_actor_cleanup_events() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cleanup_events.db");

        let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();

        // Insert old event (10 days ago)
        let old_ts = Utc::now() - chrono::Duration::days(10);
        let old_event = Event {
            id: None,
            ts: old_ts,
            source: "old.source".to_string(),
            kind: EventKind::System,
            severity: EventSeverity::Info,
            message: "Old event".to_string(),
            payload: None,
        };
        tx.send(Command::InsertEvent(old_event)).unwrap();

        // Insert recent event
        let new_event = Event {
            id: None,
            ts: Utc::now(),
            source: "new.source".to_string(),
            kind: EventKind::Alert,
            severity: EventSeverity::Critical,
            message: "New event".to_string(),
            payload: None,
        };
        tx.send(Command::InsertEvent(new_event)).unwrap();
        tx.send(Command::Checkpoint).unwrap();

        // Cleanup events older than 7 days
        tx.send(Command::CleanupEvents { retention_days: 7 })
            .unwrap();
        tx.send(Command::Checkpoint).unwrap();
        tx.send(Command::Shutdown).unwrap();

        handle.join().unwrap();

        // Verify only recent event remains
        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let source: String = conn
            .query_row("SELECT source FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(source, "new.source");
    }
}
