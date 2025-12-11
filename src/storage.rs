//! Storage Layer
//!
//! High-performance DuckDB storage with read/write separation:
//! - **Writer**: Single dedicated thread with exclusive Connection using MPSC + Appender
//! - **Reader**: r2d2 connection pool for concurrent reads
//!
//! # Components
//!
//! - [`StorageWriter`]: Unified write facade for metrics and events via MPSC channel
//! - [`MetricReader`] / [`EventReader`] / [`RawSqlReader`]: Sync read facades via pool
//! - [`StorageAdmin`]: Cleanup and maintenance operations
//! - [`StorageBuilder`] / [`StorageHandles`]: Initialization and lifecycle management

mod actor;
mod builder;
mod error;
mod facades;
mod pool;
mod schema;
mod types;

pub use builder::{StorageBuilder, StorageHandles};
pub use error::StorageError;
pub use facades::{
    EventQuery, EventReader, MetricQuery, MetricReader, MetricResult, RawSqlReader, SortOrder,
    StorageAdmin, StorageWriter,
};
pub use types::{
    DynamicTags, Event, EventKind, EventPayload, EventSeverity, EventSource, MetricCategory,
    MetricSeries, MetricValue, StaticTags,
};
