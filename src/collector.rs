//! Collector Layer
//!
//! Data collection framework with pluggable collectors that send metrics
//! to storage via MPSC channel. Each collector runs in its own Tokio task.
//!
//! # Architecture
//!
//! - [`Collector`]: Core trait for implementing data collectors
//! - [`CollectorConfig`]: Configuration trait for collector settings
//! - [`Schedule`]: Execution schedule (interval or cron)
//! - [`CollectorRegistry`]: Manages collector lifecycle and graceful shutdown
//!
//! # Example
//!
//! ```rust,no_run
//! use oculus::{Collector, TcpCollector, TcpConfig, Schedule, StorageBuilder};
//! use std::time::Duration;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let handles = StorageBuilder::new("/tmp/test.db").build()?;
//! let config = TcpConfig::new("redis-probe", "127.0.0.1:6379".parse()?)
//!     .with_schedule(Schedule::interval(Duration::from_secs(30)));
//! let collector = TcpCollector::new(config, handles.writer.clone());
//! // registry.spawn(collector);
//! # Ok(())
//! # }
//! ```

pub mod network;
mod registry;
mod traits;

pub use registry::{CollectorRegistry, DEFAULT_SHUTDOWN_TIMEOUT, JobInfo};
pub use traits::{Collector, CollectorConfig, CollectorError, Schedule};
