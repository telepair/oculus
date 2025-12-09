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
//! use oculus::{StorageBuilder, Metric};
//! use chrono::Utc;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Build storage layer (spawns writer actor thread)
//!     let handles = StorageBuilder::new("./oculus.db")
//!         .pool_size(4)
//!         .build()?;
//!
//!     // Insert a metric (sent to writer via MPSC channel)
//!     let metric = Metric {
//!         ts: Utc::now(),
//!         category: "test".to_string(),
//!         symbol: "test.metric".to_string(),
//!         value: 42.0,
//!         tags: None,
//!     };
//!     handles.metric_writer.insert(metric)?;
//!
//!     // Query metrics (via read pool)
//!     let results = handles.metric_reader.query(Default::default())?;
//!     println!("Found {} metrics", results.len());
//!
//!     // Graceful shutdown
//!     handles.shutdown()?;
//!     Ok(())
//! }
//! ```

pub mod storage;

// Re-export public types
pub use storage::{
    Event, EventReader, EventType, EventWriter, Metric, MetricReader, MetricWriter, RawSqlReader,
    Severity, StorageAdmin, StorageBuilder, StorageError, StorageHandles,
};
