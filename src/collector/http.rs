//! HTTP collectors for probing HTTP/HTTPS endpoints.
//!
//! - [`HttpCollector`]: HTTP endpoint connectivity and latency probe

mod collector;

pub use collector::{HttpCollector, HttpConfig, HttpMethod};
