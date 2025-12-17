//! Storage Layer
//!
//! High-performance SQLite storage with async read/write separation:
//! - **Writer**: Async task with exclusive writes using tokio mpsc channel
//! - **Reader**: Connection pool for concurrent reads
//!
//! # Components
//!
//! - [`StorageWriter`]: Unified write facade for metrics and events via async channel
//! - [`MetricReader`] / [`EventReader`]: Async read facades
//! - [`CollectorStore`]: Collector configuration CRUD operations
//! - [`StorageAdmin`]: Cleanup and maintenance operations
//! - [`StorageBuilder`] / [`StorageHandles`]: Initialization and lifecycle management

mod actor;
mod builder;
pub mod collector_store;
pub mod db;
mod error;
mod facades;
mod schema;
mod types;

pub use builder::{StorageBuilder, StorageHandles};
pub use collector_store::{
    CollectorRecord, CollectorSource, CollectorStore, CollectorType, SyncResult,
};
pub use error::StorageError;
pub use facades::{
    CategoryStats, EventQuery, EventReader, MetricQuery, MetricReader, MetricResult, MetricStats,
    SortOrder, StorageAdmin, StorageWriter,
};
pub use types::{
    DynamicTags, Event, EventKind, EventPayload, EventSeverity, EventSource, MetricCategory,
    MetricSeries, MetricValue, StaticTags,
};
