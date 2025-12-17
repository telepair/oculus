//! Configuration module for Oculus application.
//!
//! Provides YAML-based configuration loading and validation for:
//! - Server settings (port, bind address)
//! - Database settings (path, pool size, channel capacity)
//! - Collector definitions (type, name, target, schedule, timeout)

mod app;
mod collector;
mod validation;

pub use app::{AppConfig, DatabaseConfig, ServerConfig};
pub use collector::CollectorsConfig;
pub use validation::{ConfigError, expand_env_vars, parse_duration};

// Re-export constants
pub use app::{
    DEFAULT_CHANNEL_CAPACITY, DEFAULT_CHECKPOINT_INTERVAL, DEFAULT_INTERVAL, DEFAULT_POOL_SIZE,
    DEFAULT_SYNC_INTERVAL, MIN_SYNC_INTERVAL,
};
