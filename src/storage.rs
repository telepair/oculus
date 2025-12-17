//! Storage Layer
//!
//! High-performance DuckDB storage with read/write separation:
//! - **Writer**: Single dedicated thread with exclusive Connection using MPSC + Appender
//! - **Reader**: Connection pool using try_clone() for concurrent reads
//!
//! # Components
//!
//! - [`StorageWriter`]: Unified write facade for metrics and events via MPSC channel
//! - [`MetricReader`] / [`EventReader`] / [`RawSqlReader`]: Sync read facades via pool
//! - [`CollectorStore`]: Collector configuration CRUD and sync operations
//! - [`StorageAdmin`]: Cleanup and maintenance operations
//! - [`StorageBuilder`] / [`StorageHandles`]: Initialization and lifecycle management

mod actor;
mod builder;
pub mod collector_store;
mod error;
mod facades;
mod pool;
mod schema;
mod types;

pub use builder::{StorageBuilder, StorageHandles};
pub use collector_store::{
    CollectorRecord, CollectorSource, CollectorStore, CollectorType, SyncResult,
};
pub use error::StorageError;
pub use facades::{
    CategoryStats, EventQuery, EventReader, MetricQuery, MetricReader, MetricResult, MetricStats,
    RawSqlReader, SortOrder, StorageAdmin, StorageWriter,
};
pub use types::{
    DynamicTags, Event, EventKind, EventPayload, EventSeverity, EventSource, MetricCategory,
    MetricSeries, MetricValue, StaticTags,
};
