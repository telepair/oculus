//! Collector Layer
//!
//! Data collection framework with pluggable collectors that send metrics
//! to storage via MPSC channel. Each collector runs in its own Tokio task.
//!
//! # Architecture
//!
//! - [`Collector`]: Core trait for implementing data collectors
//! - [`Schedule`]: Execution schedule (interval or cron)
//! - [`CollectorRegistry`]: Manages collector lifecycle and graceful shutdown
//!
//! # Example
//!
//! ```rust,no_run
//! use oculus::{TcpCollector, TcpConfig, StorageBuilder};
//! use std::time::Duration;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let handles = StorageBuilder::new("/tmp/test.db").build()?;
//! let config = TcpConfig::new("redis-probe", "127.0.0.1", 6379)
//!     .with_interval(Duration::from_secs(30));
//! let collector = TcpCollector::new(config, handles.writer.clone());
//! // registry.spawn(collector);
//! # Ok(())
//! # }
//! ```

mod registry;
pub mod tcp;
mod traits;

pub use registry::{CollectorRegistry, JobInfo};
pub use traits::{Collector, CollectorError, Schedule};
