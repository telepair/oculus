//! Oculus - Unified Telemetry Library
//!
//! This crate provides the core functionality for the Oculus monitoring system.
//! It can be used as a library by other Rust projects, or run as a standalone
//! binary with the `oculus` executable.
//!
//! # Architecture
//!
//! - **Collectors**: Data collection from various sources (network, crypto, stock, prediction markets)
//! - **Storage**: DuckDB-based persistence layer with read/write separation
//! - **Rule Engine**: Simple (YAML) and complex (SQL) rule evaluation
//! - **Presentation**: Web UI and REST API
//! - **Notification**: Multi-channel alert delivery
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use oculus::{StorageBuilder, MetricSeries, MetricValue, MetricCategory, StaticTags};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Build storage layer
//!     let handles = StorageBuilder::new("./oculus.db").build()?;
//!
//!     // Create and insert a metric
//!     let series = MetricSeries::new(
//!         MetricCategory::NetworkTcp,
//!         "latency",
//!         "127.0.0.1:6379",
//!         StaticTags::new(),
//!         Some("Redis latency".to_string()),
//!     );
//!     let value = MetricValue::new(series.series_id, 42.0, true);
//!     handles.writer.upsert_metric_series(series)?;
//!     handles.writer.insert_metric_value(value)?;
//!
//!     // Query metrics
//!     let results = handles.metric_reader.query(Default::default())?;
//!     println!("Found {} metrics", results.len());
//!
//!     handles.shutdown()?;
//!     Ok(())
//! }
//! ```

pub mod collector;
pub mod config;
pub mod server;
pub mod storage;

// Re-export storage types
pub use storage::{
    DynamicTags, Event, EventKind, EventPayload, EventQuery, EventReader, EventSeverity,
    EventSource, MetricCategory, MetricQuery, MetricReader, MetricResult, MetricSeries,
    MetricValue, RawSqlReader, SortOrder, StaticTags, StorageAdmin, StorageBuilder, StorageError,
    StorageHandles, StorageWriter,
};

pub use collector::{
    Collector, CollectorError, CollectorRegistry, JobInfo, Schedule,
    tcp::{TcpCollector, TcpConfig},
};
