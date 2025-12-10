//! Network collectors for probing network endpoints.
//!
//! - [`TcpCollector`]: TCP port connectivity and latency probe

mod tcp;

pub use tcp::{TcpCollector, TcpConfig};
