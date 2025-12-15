//! Storage builder and handles.
//!
//! Provides a builder pattern for constructing the storage layer
//! and a handles struct for accessing all storage facades.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::storage::StorageError;
use crate::storage::actor::{DEFAULT_BATCH_FLUSH_INTERVAL, DEFAULT_BATCH_SIZE, DbActor};
use crate::storage::pool::ReadPool;
use crate::storage::{EventReader, MetricReader, RawSqlReader, StorageAdmin, StorageWriter};

/// Default channel capacity for writer commands.
///
/// With batch flushing every 500 items or 1 second, this capacity supports:
/// - Up to 10,000 queued metrics before blocking
/// - Approximately 20 seconds of buffering at 500 metrics/sec
const DEFAULT_CHANNEL_CAPACITY: usize = 10_000;

/// Minimum connection pool size.
const MIN_POOL_SIZE: u32 = 2;

/// Maximum connection pool size.
const MAX_POOL_SIZE: u32 = 32;

/// Calculate default pool size based on available CPU parallelism.
///
/// Returns the number of available CPUs, clamped between MIN_POOL_SIZE and MAX_POOL_SIZE.
fn default_pool_size() -> u32 {
    std::thread::available_parallelism()
        .map(|p| (p.get() as u32).clamp(MIN_POOL_SIZE, MAX_POOL_SIZE))
        .unwrap_or(4)
}

/// Default WAL checkpoint interval.
const DEFAULT_CHECKPOINT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Builder for constructing the storage layer.
pub struct StorageBuilder {
    db_path: PathBuf,
    pool_size: u32,
    channel_capacity: usize,
    checkpoint_interval: std::time::Duration,
    batch_size: usize,
    batch_flush_interval: std::time::Duration,
}

impl StorageBuilder {
    /// Create a new storage builder.
    ///
    /// Pool size defaults to the number of available CPUs (clamped to 2-32).
    pub fn new(db_path: impl AsRef<Path>) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
            pool_size: default_pool_size(),
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
            checkpoint_interval: DEFAULT_CHECKPOINT_INTERVAL,
            batch_size: DEFAULT_BATCH_SIZE,
            batch_flush_interval: DEFAULT_BATCH_FLUSH_INTERVAL,
        }
    }

    /// Set the connection pool size for readers.
    pub fn pool_size(mut self, size: u32) -> Self {
        self.pool_size = size;
        self
    }

    /// Set the channel capacity for writer commands.
    pub fn channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = capacity;
        self
    }

    /// Set the WAL checkpoint interval.
    pub fn checkpoint_interval(mut self, interval: std::time::Duration) -> Self {
        self.checkpoint_interval = interval;
        self
    }

    /// Set the batch size for metric value buffering.
    ///
    /// The actor will flush buffered metric values when this threshold is reached.
    /// Default: 500 items.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the batch flush interval for metric value buffering.
    ///
    /// The actor will flush buffered metric values after this duration, even if
    /// the batch size threshold hasn't been reached. Default: 1 second.
    pub fn batch_flush_interval(mut self, interval: std::time::Duration) -> Self {
        self.batch_flush_interval = interval;
        self
    }

    /// Build the storage layer and return handles.
    pub fn build(self) -> Result<StorageHandles, StorageError> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = self.db_path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::Internal(format!(
                    "Failed to create database directory '{}': {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        // Spawn writer actor - returns a cloneable connection for readers
        let (actor_handle, tx, reader_conn) = DbActor::spawn(
            &self.db_path,
            self.channel_capacity,
            self.checkpoint_interval,
            self.batch_size,
            self.batch_flush_interval,
        )?;

        // Create reader pool from the cloned connection.
        // This ensures readers share the same database instance as the writer,
        // enabling immediate write visibility without WAL checkpoint delays.
        let pool = ReadPool::new(reader_conn);

        Ok(StorageHandles {
            writer: StorageWriter::new(tx.clone()),
            metric_reader: MetricReader::new(Arc::clone(&pool)),
            event_reader: EventReader::new(Arc::clone(&pool)),
            raw_sql_reader: RawSqlReader::new(pool),
            admin: StorageAdmin::new(tx),
            actor_handle: Some(actor_handle),
        })
    }
}

/// Handles to all storage layer facades.
pub struct StorageHandles {
    /// Unified writer facade for metrics and events.
    pub writer: StorageWriter,
    /// Facade for reading metrics.
    pub metric_reader: MetricReader,
    /// Facade for reading events.
    pub event_reader: EventReader,
    /// Facade for executing raw SQL queries.
    pub raw_sql_reader: RawSqlReader,
    /// Facade for storage administration.
    pub admin: StorageAdmin,
    /// Internal actor handle for graceful shutdown.
    actor_handle: Option<JoinHandle<()>>,
}

impl StorageHandles {
    /// Gracefully shutdown the storage layer.
    ///
    /// Sends shutdown command to the writer actor and waits for it to finish.
    pub fn shutdown(mut self) -> Result<(), StorageError> {
        self.admin.shutdown()?;

        if let Some(handle) = self.actor_handle.take() {
            handle
                .join()
                .map_err(|_| StorageError::Internal("Failed to join actor thread".to_string()))?;
        }

        Ok(())
    }
}

impl Drop for StorageHandles {
    fn drop(&mut self) {
        // Try graceful shutdown if not already done
        if self.actor_handle.is_some() {
            let _ = self.admin.shutdown();
            if let Some(handle) = self.actor_handle.take() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::types::{MetricCategory, MetricSeries, MetricValue, StaticTags};
    use tempfile::tempdir;

    #[test]
    fn test_storage_builder() {
        use crate::storage::actor::DbActor;
        use crate::storage::facades::{MetricQuery, StorageAdmin, StorageWriter};

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Phase 1: Write using actor directly
        {
            let (handle, tx, _reader_conn) = DbActor::spawn(
                &db_path,
                100,
                std::time::Duration::from_secs(1),
                DEFAULT_BATCH_SIZE,
                DEFAULT_BATCH_FLUSH_INTERVAL,
            )
            .unwrap();
            let writer = StorageWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let series = MetricSeries::new(
                MetricCategory::Custom,
                "test",
                "builder.test",
                StaticTags::new(),
                None,
            );
            let value = MetricValue::new(series.series_id, 123.0, true);

            writer.upsert_metric_series(series).unwrap();
            writer.insert_metric_value(value).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        // Phase 2: Read using StorageBuilder
        let handles = StorageBuilder::new(&db_path).build().unwrap();
        let results = handles
            .metric_reader
            .query(MetricQuery {
                target: Some("builder.test".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, 123.0);

        handles.shutdown().unwrap();
    }

    #[test]
    fn test_storage_roundtrip() {
        use crate::storage::actor::DbActor;
        use crate::storage::facades::{MetricQuery, StorageAdmin, StorageWriter};

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("roundtrip.db");

        // Phase 1: Write using actor directly
        {
            let (handle, tx, _reader_conn) = DbActor::spawn(
                &db_path,
                100,
                std::time::Duration::from_secs(1),
                DEFAULT_BATCH_SIZE,
                DEFAULT_BATCH_FLUSH_INTERVAL,
            )
            .unwrap();
            let writer = StorageWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            for i in 0..5 {
                let series = MetricSeries::new(
                    MetricCategory::Custom,
                    "roundtrip",
                    format!("target.{i}"),
                    StaticTags::new(),
                    None,
                );
                let value = MetricValue::new(series.series_id, f64::from(i), true);
                writer.upsert_metric_series(series).unwrap();
                writer.insert_metric_value(value).unwrap();
            }

            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        // Phase 2: Read using StorageBuilder
        let handles = StorageBuilder::new(&db_path).build().unwrap();
        let results = handles
            .metric_reader
            .query(MetricQuery {
                name: Some("roundtrip".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 5);

        handles.shutdown().unwrap();
    }

    #[test]
    fn test_default_pool_size_within_bounds() {
        let size = super::default_pool_size();
        assert!(size >= super::MIN_POOL_SIZE);
        assert!(size <= super::MAX_POOL_SIZE);
    }
}
