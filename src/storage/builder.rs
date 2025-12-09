//! Storage builder and handles.
//!
//! Provides a builder pattern for constructing the storage layer
//! and a handles struct for accessing all storage facades.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::storage::StorageError;
use crate::storage::actor::DbActor;
use crate::storage::facades::{
    EventReader, EventWriter, MetricReader, MetricWriter, RawSqlReader, StorageAdmin,
};
use crate::storage::pool::ReadPool;

/// Default channel capacity for writer commands.
const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Default connection pool size for readers.
const DEFAULT_POOL_SIZE: u32 = 4;

/// Builder for constructing the storage layer.
pub struct StorageBuilder {
    db_path: PathBuf,
    pool_size: u32,
    channel_capacity: usize,
}

impl StorageBuilder {
    /// Create a new storage builder.
    pub fn new(db_path: impl AsRef<Path>) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
            pool_size: DEFAULT_POOL_SIZE,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
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

    /// Build the storage layer and return handles.
    pub fn build(self) -> Result<StorageHandles, StorageError> {
        // Spawn writer actor
        let (actor_handle, tx) = DbActor::spawn(&self.db_path, self.channel_capacity)?;

        // Create reader pool
        let pool = ReadPool::new(&self.db_path, self.pool_size)?;

        Ok(StorageHandles {
            metric_writer: MetricWriter::new(tx.clone()),
            event_writer: EventWriter::new(tx.clone()),
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
    /// Facade for writing metrics.
    pub metric_writer: MetricWriter,
    /// Facade for writing events.
    pub event_writer: EventWriter,
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
    use crate::storage::facades::MetricQuery;
    use crate::storage::types::Metric;
    use chrono::Utc;
    use tempfile::tempdir;

    #[test]
    fn test_storage_builder() {
        use crate::storage::actor::DbActor;
        use crate::storage::facades::{MetricWriter, StorageAdmin};

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Phase 1: Write using actor directly (no ReadPool during write)
        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = MetricWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let metric = Metric {
                ts: Utc::now(),
                category: "test".to_string(),
                symbol: "builder.test".to_string(),
                value: 123.0,
                tags: None,
            };
            writer.insert(metric).unwrap();
            admin.checkpoint().unwrap();
            admin.shutdown().unwrap();
            handle.join().unwrap();
        }

        // Phase 2: Read using StorageBuilder (creates fresh connections)
        let handles = StorageBuilder::new(&db_path).build().unwrap();
        let results = handles
            .metric_reader
            .query(MetricQuery {
                symbol: Some("builder.test".to_string()),
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
        use crate::storage::facades::{MetricWriter, StorageAdmin};

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("roundtrip.db");

        // Phase 1: Write using actor directly
        {
            let (handle, tx) = DbActor::spawn(&db_path, 100).unwrap();
            let writer = MetricWriter::new(tx.clone());
            let admin = StorageAdmin::new(tx);

            let metrics: Vec<Metric> = (0..5)
                .map(|i| Metric {
                    ts: Utc::now(),
                    category: "roundtrip".to_string(),
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

        // Phase 2: Read using StorageBuilder
        let handles = StorageBuilder::new(&db_path).build().unwrap();
        let results = handles
            .metric_reader
            .query(MetricQuery {
                category: Some("roundtrip".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 5);

        handles.shutdown().unwrap();
    }
}
