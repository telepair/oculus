//! Configuration module for Oculus application.
//!
//! Provides YAML-based configuration loading and validation for:
//! - Server settings (port, bind address)
//! - Database settings (path, pool size, channel capacity)
//! - Collector definitions (type, name, target, schedule, timeout)

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

use crate::storage::MetricCategory;

/// Configuration error types.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read configuration file.
    #[error("failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    /// Failed to parse YAML configuration.
    #[error("failed to parse YAML config: {0}")]
    ParseError(#[from] serde_yaml::Error),

    /// Configuration validation failed.
    #[error("config validation error: {0}")]
    ValidationError(String),
}

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Web server configuration.
    pub server: ServerConfig,

    /// Database configuration.
    pub database: DatabaseConfig,

    /// Collector configurations.
    #[serde(default)]
    pub collectors: Vec<CollectorConfigEntry>,
}

/// Web server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server bind address (default: "0.0.0.0").
    #[serde(default = "default_bind_address")]
    pub bind: String,

    /// Server port (default: 8080).
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

/// Database configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database file path.
    pub path: String,

    /// Connection pool size for read operations (default: 4).
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// MPSC channel capacity for write operations (default: 1000).
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,

    /// WAL checkpoint interval (default: "5s").
    #[serde(default = "default_checkpoint_interval")]
    pub checkpoint_interval: String,
}

fn default_pool_size() -> u32 {
    4
}

fn default_channel_capacity() -> usize {
    10_000
}

fn default_checkpoint_interval() -> String {
    "5s".to_string()
}

/// Collector configuration entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorConfigEntry {
    /// Collector type - maps to MetricCategory (e.g., "network.tcp", "crypto").
    #[serde(rename = "type")]
    pub collector_type: MetricCategory,

    /// Unique collector name.
    pub name: String,

    /// Target endpoint (e.g., "127.0.0.1:6379", "<https://api.example.com>").
    pub target: String,

    /// Collection interval (e.g., "30s", "1m", "5m"). Mutually exclusive with `cron`.
    #[serde(default)]
    pub interval: Option<String>,

    /// Cron expression for scheduled execution (6-field: sec min hour day month weekday).
    /// Mutually exclusive with `interval`.
    #[serde(default)]
    pub cron: Option<String>,

    /// Timeout for each probe (e.g., "5s").
    pub timeout: String,

    /// Static tags for this collector (key-value pairs).
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

impl AppConfig {
    /// Load configuration from a YAML file.
    ///
    /// # Errors
    /// Returns `ConfigError` if the file cannot be read, parsed, or validated.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let config: Self = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration values.
    ///
    /// # Errors
    /// Returns `ConfigError::ValidationError` if any field is invalid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate server port
        if self.server.port == 0 {
            return Err(ConfigError::ValidationError(
                "server port must be non-zero".to_string(),
            ));
        }

        // Validate database pool size
        if self.database.pool_size == 0 {
            return Err(ConfigError::ValidationError(
                "database pool_size must be positive".to_string(),
            ));
        }

        // Validate channel capacity
        if self.database.channel_capacity == 0 {
            return Err(ConfigError::ValidationError(
                "database channel_capacity must be positive".to_string(),
            ));
        }

        // Validate checkpoint interval
        parse_duration(&self.database.checkpoint_interval).map_err(|e| {
            ConfigError::ValidationError(format!("database checkpoint_interval: {}", e))
        })?;

        // Validate collectors
        for collector in &self.collectors {
            if collector.name.is_empty() {
                return Err(ConfigError::ValidationError(
                    "collector name cannot be empty".to_string(),
                ));
            }

            if collector.target.is_empty() {
                return Err(ConfigError::ValidationError(format!(
                    "collector '{}': target cannot be empty",
                    collector.name
                )));
            }

            // Validate schedule: either interval or cron must be set (not both)
            match (&collector.interval, &collector.cron) {
                (Some(interval), None) => {
                    parse_duration(interval).map_err(|e| {
                        ConfigError::ValidationError(format!(
                            "collector '{}': invalid interval format: {}",
                            collector.name, e
                        ))
                    })?;
                }
                (None, Some(cron_expr)) => {
                    // Validate cron expression
                    use std::str::FromStr;
                    cron::Schedule::from_str(cron_expr).map_err(|e| {
                        ConfigError::ValidationError(format!(
                            "collector '{}': invalid cron expression: {}",
                            collector.name, e
                        ))
                    })?;
                }
                (Some(_), Some(_)) => {
                    return Err(ConfigError::ValidationError(format!(
                        "collector '{}': cannot specify both 'interval' and 'cron'",
                        collector.name
                    )));
                }
                (None, None) => {
                    return Err(ConfigError::ValidationError(format!(
                        "collector '{}': must specify either 'interval' or 'cron'",
                        collector.name
                    )));
                }
            }

            // Validate timeout format
            parse_duration(&collector.timeout).map_err(|e| {
                ConfigError::ValidationError(format!(
                    "collector '{}': invalid timeout format: {}",
                    collector.name, e
                ))
            })?;
        }

        Ok(())
    }
}

/// Parse duration string (e.g., "30s", "1m", "5m").
///
/// Supports units: `s` (seconds), `m` (minutes), `h` (hours).
///
/// # Examples
///
/// ```
/// use oculus::config::parse_duration;
///
/// assert_eq!(parse_duration("30s").unwrap().as_secs(), 30);
/// assert_eq!(parse_duration("1m").unwrap().as_secs(), 60);
/// assert_eq!(parse_duration("2h").unwrap().as_secs(), 7200);
/// ```
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("duration string is empty".to_string());
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number: {}", num_str))?;

    match unit {
        "s" => Ok(Duration::from_secs(num)),
        "m" => Ok(Duration::from_secs(num * 60)),
        "h" => Ok(Duration::from_secs(num * 3600)),
        _ => Err(format!("invalid unit: {}. Use 's', 'm', or 'h'", unit)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_valid() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("30x").is_err());
        assert!(parse_duration("30").is_err());
    }

    #[test]
    fn test_config_validation_valid() {
        let config = AppConfig {
            server: ServerConfig {
                bind: "127.0.0.1".to_string(),
                port: 8080,
            },
            database: DatabaseConfig {
                path: "./test.db".to_string(),
                pool_size: 4,
                channel_capacity: 1000,
                checkpoint_interval: "5s".to_string(),
            },
            collectors: vec![CollectorConfigEntry {
                collector_type: MetricCategory::NetworkTcp,
                name: "test-probe".to_string(),
                target: "127.0.0.1:6379".to_string(),
                interval: Some("30s".to_string()),
                cron: None,
                timeout: "5s".to_string(),
                tags: BTreeMap::new(),
            }],
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validation_invalid_port() {
        let config = AppConfig {
            server: ServerConfig {
                bind: "0.0.0.0".to_string(),
                port: 0,
            },
            database: DatabaseConfig {
                path: "./test.db".to_string(),
                pool_size: 4,
                channel_capacity: 1000,
                checkpoint_interval: "5s".to_string(),
            },
            collectors: vec![],
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_invalid_pool_size() {
        let config = AppConfig {
            server: ServerConfig {
                bind: "0.0.0.0".to_string(),
                port: 8080,
            },
            database: DatabaseConfig {
                path: "./test.db".to_string(),
                pool_size: 0,
                channel_capacity: 1000,
                checkpoint_interval: "5s".to_string(),
            },
            collectors: vec![],
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_invalid_interval() {
        let config = AppConfig {
            server: ServerConfig {
                bind: "0.0.0.0".to_string(),
                port: 8080,
            },
            database: DatabaseConfig {
                path: "./test.db".to_string(),
                pool_size: 4,
                channel_capacity: 1000,
                checkpoint_interval: "5s".to_string(),
            },
            collectors: vec![CollectorConfigEntry {
                collector_type: MetricCategory::NetworkTcp,
                name: "test".to_string(),
                target: "127.0.0.1:6379".to_string(),
                interval: Some("invalid".to_string()),
                cron: None,
                timeout: "5s".to_string(),
                tags: BTreeMap::new(),
            }],
        };

        assert!(config.validate().is_err());
    }
}
