//! Application configuration structures.

use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::storage::{CollectorRecord, CollectorType};

use super::collector::CollectorsConfig;
use super::validation::{ConfigError, parse_duration};

// =============================================================================
// Constants
// =============================================================================

/// Default collection interval (30 seconds).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Default connection pool size.
pub const DEFAULT_POOL_SIZE: u32 = 4;

/// Default channel capacity.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 10_000;

/// Default checkpoint interval (5 seconds).
pub const DEFAULT_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5);

/// Default collector sync interval (10 seconds).
pub const DEFAULT_SYNC_INTERVAL: Duration = Duration::from_secs(10);

/// Minimum collector sync interval (1 second).
pub const MIN_SYNC_INTERVAL: Duration = Duration::from_secs(1);

fn default_pool_size() -> u32 {
    DEFAULT_POOL_SIZE
}

fn default_channel_capacity() -> usize {
    DEFAULT_CHANNEL_CAPACITY
}

fn default_checkpoint_interval() -> String {
    "5s".to_string()
}

fn default_sync_interval() -> Duration {
    DEFAULT_SYNC_INTERVAL
}

// =============================================================================
// Server Configuration
// =============================================================================

/// Web server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Server bind address (default: "0.0.0.0").
    pub bind: String,

    /// Server port (default: 8080).
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".to_string(),
            port: 8080,
        }
    }
}

// =============================================================================
// Database Configuration
// =============================================================================

/// Database configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database file path.
    pub path: String,

    /// Connection pool size for read operations (default: 4).
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// MPSC channel capacity for write operations (default: 10000).
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,

    /// WAL checkpoint interval (default: "5s").
    #[serde(default = "default_checkpoint_interval")]
    pub checkpoint_interval: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: "oculus.db".to_string(),
            pool_size: DEFAULT_POOL_SIZE,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
            checkpoint_interval: "5s".to_string(),
        }
    }
}

// =============================================================================
// Application Configuration
// =============================================================================

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Web server configuration.
    pub server: ServerConfig,

    /// Database configuration.
    pub database: DatabaseConfig,

    /// Collector configurations grouped by type.
    #[serde(default)]
    pub collectors: CollectorsConfig,

    /// Path to a directory with additional collector config files.
    #[serde(default)]
    pub collector_path: Option<String>,

    /// Collector sync interval (default: 10s, minimum: 1s).
    #[serde(default = "default_sync_interval", with = "humantime_serde")]
    pub collector_sync_interval: Duration,
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
        // Validate server bind address
        self.server.bind.parse::<IpAddr>().map_err(|_| {
            ConfigError::ValidationError(format!(
                "invalid server bind address: '{}'",
                self.server.bind
            ))
        })?;

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
        self.collectors.validate()?;

        Ok(())
    }

    /// Load configuration including collector_path directory.
    ///
    /// If `collector_path` is specified, scans the directory for YAML files
    /// and merges their collector configurations.
    pub fn load_with_collector_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let mut config = Self::load(path)?;

        if let Some(ref collector_dir) = config.collector_path {
            let additional = CollectorsConfig::load_from_dir(collector_dir)?;
            config.collectors = config.collectors.merge(additional);
        }

        config.validate()?;
        Ok(config)
    }

    /// Convert collectors config to CollectorRecords for database sync.
    pub fn to_collector_records(&self) -> Vec<CollectorRecord> {
        let mut records = Vec::new();

        for tcp in &self.collectors.tcp {
            let config_json = serde_json::to_value(tcp).unwrap_or_default();
            records.push(CollectorRecord::from_config(
                CollectorType::Tcp,
                &tcp.name,
                tcp.enabled,
                &tcp.group,
                config_json,
            ));
        }

        for ping in &self.collectors.ping {
            let config_json = serde_json::to_value(ping).unwrap_or_default();
            records.push(CollectorRecord::from_config(
                CollectorType::Ping,
                &ping.name,
                ping.enabled,
                &ping.group,
                config_json,
            ));
        }

        for http in &self.collectors.http {
            let config_json = serde_json::to_value(http).unwrap_or_default();
            records.push(CollectorRecord::from_config(
                CollectorType::Http,
                &http.name,
                http.enabled,
                &http.group,
                config_json,
            ));
        }

        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::tcp::TcpConfig;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.bind, "0.0.0.0");
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn test_database_config_default() {
        let config = DatabaseConfig::default();
        assert_eq!(config.path, "oculus.db");
        assert_eq!(config.pool_size, DEFAULT_POOL_SIZE);
        assert_eq!(config.channel_capacity, DEFAULT_CHANNEL_CAPACITY);
        assert_eq!(config.checkpoint_interval, "5s");
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
            collectors: CollectorsConfig {
                tcp: vec![TcpConfig::new("test-probe", "127.0.0.1", 6379)],
                ping: vec![],
                http: vec![],
            },
            collector_path: None,
            collector_sync_interval: DEFAULT_SYNC_INTERVAL,
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
            database: DatabaseConfig::default(),
            collectors: CollectorsConfig::default(),
            collector_path: None,
            collector_sync_interval: DEFAULT_SYNC_INTERVAL,
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_invalid_bind_address() {
        let config = AppConfig {
            server: ServerConfig {
                bind: "not-an-ip".to_string(),
                port: 8080,
            },
            database: DatabaseConfig::default(),
            collectors: CollectorsConfig::default(),
            collector_path: None,
            collector_sync_interval: DEFAULT_SYNC_INTERVAL,
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid server bind address")
        );
    }
}
