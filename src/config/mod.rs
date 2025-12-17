//! Configuration module for Oculus application.
//!
//! Provides YAML-based configuration loading and validation for:
//! - Server settings (port, bind address)
//! - Database settings (path, channel capacity)
//! - Collector include directory

mod app;
mod collector;
mod validation;

pub use app::{AppConfig, DatabaseConfig, DatabaseDriver, ServerConfig};
pub use collector::CollectorsConfig;
pub use validation::{ConfigError, expand_env_vars, parse_duration};

// Re-export constants
pub use app::{DEFAULT_CHANNEL_CAPACITY, DEFAULT_INTERVAL};
