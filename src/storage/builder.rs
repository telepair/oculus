//! Storage builder and handles.
//!
//! Provides a builder pattern for constructing the storage layer
//! and a handles struct for accessing all storage facades.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::storage::StorageError;
use crate::storage::actor::{
    Command, DEFAULT_BATCH_FLUSH_INTERVAL, DEFAULT_BATCH_SIZE, DEFAULT_CHANNEL_CAPACITY, DbActor,
};
use crate::storage::collector_store::CollectorStore;
use crate::storage::db::SqlitePool;
use crate::storage::facades::{EventReader, MetricReader, StorageAdmin, StorageWriter};

/// Builder for constructing the storage layer.
pub struct StorageBuilder {
    db_url: String,
    channel_capacity: usize,
    batch_size: usize,
    batch_flush_interval: Duration,
}

impl StorageBuilder {
    /// Create a new storage builder.
    ///
    /// # Arguments
    ///
    /// * `db_url` - SQLite connection URL, e.g., `sqlite:data/oculus.db?mode=rwc`
    pub fn new(db_url: impl Into<String>) -> Self {
        Self {
            db_url: db_url.into(),
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
            batch_size: DEFAULT_BATCH_SIZE,
            batch_flush_interval: DEFAULT_BATCH_FLUSH_INTERVAL,
        }
    }

    /// Set the channel capacity for writer commands.
    pub fn channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = capacity;
        self
    }

    /// Set the batch size for metric value buffering.
    ///
    /// The actor will flush buffered metric values when this threshold is reached.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the batch flush interval for metric value buffering.
    ///
    /// The actor will flush buffered metric values after this duration, even if
    /// the batch size threshold hasn't been reached.
    pub fn batch_flush_interval(mut self, interval: Duration) -> Self {
        self.batch_flush_interval = interval;
        self
    }

    /// Build the storage layer and return handles.
    pub async fn build(self) -> Result<StorageHandles, StorageError> {
        let (handle, tx, pool) = DbActor::spawn(
            &self.db_url,
            self.channel_capacity,
            self.batch_size,
            self.batch_flush_interval,
        )
        .await?;

        Ok(StorageHandles {
            writer: StorageWriter::new(tx.clone()),
            metric_reader: MetricReader::new(pool.clone()),
            event_reader: EventReader::new(pool.clone()),
            collector_store: CollectorStore::new(pool.clone()),
            admin: StorageAdmin::new(tx.clone()),
            _actor_handle: Some(handle),
            _actor_tx: tx,
            _pool: pool,
        })
    }
}

/// Handles to all storage layer facades.
pub struct StorageHandles {
    pub writer: StorageWriter,
    pub metric_reader: MetricReader,
    pub event_reader: EventReader,
    pub collector_store: CollectorStore,
    pub admin: StorageAdmin,
    _actor_handle: Option<JoinHandle<()>>,
    _actor_tx: Sender<Command>,
    _pool: Arc<SqlitePool>,
}

impl StorageHandles {
    /// Gracefully shutdown the storage layer.
    ///
    /// Sends shutdown command to the writer actor and waits for it to finish.
    pub async fn shutdown(mut self) -> Result<(), StorageError> {
        self.admin.shutdown()?;
        if let Some(handle) = self._actor_handle.take() {
            handle
                .await
                .map_err(|e| StorageError::Internal(e.to_string()))?;
        }
        self._pool.close().await;
        Ok(())
    }
}

impl Drop for StorageHandles {
    fn drop(&mut self) {
        // Try to send shutdown if actor is still running
        if self._actor_handle.is_some() {
            let _ = self._actor_tx.try_send(Command::Shutdown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::facades::MetricQuery;
    use crate::storage::types::{MetricCategory, MetricSeries, MetricValue, StaticTags};

    #[tokio::test]
    async fn test_storage_builder() {
        let handles = StorageBuilder::new("sqlite::memory:")
            .channel_capacity(100)
            .batch_size(10)
            .batch_flush_interval(Duration::from_millis(100))
            .build()
            .await
            .unwrap();

        // Verify all handles are accessible
        let _ = handles.writer.dropped_metrics();
        handles.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_storage_roundtrip() {
        let handles = StorageBuilder::new("sqlite::memory:")
            .batch_size(10)
            .batch_flush_interval(Duration::from_millis(100))
            .build()
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

        handles.writer.upsert_metric_series(series).unwrap();
        handles.writer.insert_metric_value(value).unwrap();
        handles.writer.flush().unwrap();

        // Wait for actor to process
        tokio::time::sleep(Duration::from_millis(200)).await;

        let results = handles
            .metric_reader
            .query(MetricQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "latency");
        assert_eq!(results[0].value, 42.5);

        handles.shutdown().await.unwrap();
    }
}
