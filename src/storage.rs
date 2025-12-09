//! Storage Layer
//!
//! High-performance DuckDB storage with read/write separation:
//! - **Writer**: Single dedicated thread with exclusive Connection using MPSC + Appender
//! - **Reader**: r2d2 connection pool for concurrent reads
//!
//! # Components
//!
//! - [`MetricWriter`] / [`EventWriter`]: Async write facades via MPSC channel
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
    EventReader, EventWriter, MetricReader, MetricWriter, RawSqlReader, StorageAdmin,
};
pub use types::{Event, EventType, Metric, Severity};
