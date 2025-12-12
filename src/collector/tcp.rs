//! TCP collectors for probing TCP endpoints.
//!
//! - [`TcpCollector`]: TCP port connectivity and latency probe

mod collector;

pub use collector::{TcpCollector, TcpConfig};
