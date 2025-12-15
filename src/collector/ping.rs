//! Ping collectors for probing network reachability via ICMP.
//!
//! - [`PingCollector`]: ICMP ping probe for host reachability and latency

mod collector;

pub use collector::{PingCollector, PingConfig};
