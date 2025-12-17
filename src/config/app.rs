//! Application configuration structures.

use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::validation::ConfigError;

// =============================================================================
// Constants
// =============================================================================

/// Default collection interval (30 seconds).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Default channel capacity.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 10_000;

fn default_channel_capacity() -> usize {
    DEFAULT_CHANNEL_CAPACITY
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

/// Supported database drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseDriver {
    /// SQLite database (embedded).
    #[default]
    Sqlite,
    /// PostgreSQL database.
    #[serde(alias = "pg", alias = "postgres")]
    Postgresql,
}

impl std::fmt::Display for DatabaseDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseDriver::Sqlite => write!(f, "sqlite"),
            DatabaseDriver::Postgresql => write!(f, "postgresql"),
        }
    }
}

/// Database configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database driver type (sqlite or postgresql).
    #[serde(default)]
    pub driver: DatabaseDriver,

    /// Database connection DSN (Data Source Name).
    ///
    /// For SQLite: `sqlite:data/oculus.db?mode=rwc` or just `data/oculus.db`
    /// For PostgreSQL: `postgres://user:pass@host:5432/dbname`
    pub dsn: String,

    /// MPSC channel capacity for write operations (default: 10000).
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            driver: DatabaseDriver::default(),
            dsn: "sqlite:oculus.db?mode=rwc".to_string(),
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }
}

impl DatabaseConfig {
    /// Get the full database connection URL with proper prefix.
    ///
    /// For SQLite, ensures the URL has the `sqlite:` prefix and `?mode=rwc` suffix.
    /// For PostgreSQL, returns the URL as-is.
    pub fn connection_url(&self) -> String {
        match self.driver {
            DatabaseDriver::Sqlite => {
                let dsn = self.dsn.trim();
                // If already has sqlite: prefix, return as-is
                if dsn.starts_with("sqlite:") {
                    dsn.to_string()
                } else {
                    // Add prefix and mode=rwc if not present
                    let base = format!("sqlite:{}", dsn);
                    if base.contains('?') {
                        base
                    } else {
                        format!("{}?mode=rwc", base)
                    }
                }
            }
            DatabaseDriver::Postgresql => self.dsn.clone(),
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

    /// Path to a directory with collector config files (include directory).
    /// Collectors from this directory are synced to database on startup.
    #[serde(default)]
    pub collector_include: Option<String>,
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

        // Validate channel capacity
        if self.database.channel_capacity == 0 {
            return Err(ConfigError::ValidationError(
                "database channel_capacity must be positive".to_string(),
            ));
        }

        // Validate collector_include path exists if specified
        if let Some(ref path) = self.collector_include {
            let p = Path::new(path);
            if !p.exists() {
                return Err(ConfigError::ValidationError(format!(
                    "collector_include path '{}' does not exist",
                    path
                )));
            }
            if !p.is_dir() {
                return Err(ConfigError::ValidationError(format!(
                    "collector_include path '{}' is not a directory",
                    path
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.bind, "0.0.0.0");
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn test_database_config_default() {
        let config = DatabaseConfig::default();
        assert_eq!(config.driver, DatabaseDriver::Sqlite);
        assert_eq!(config.dsn, "sqlite:oculus.db?mode=rwc");
        assert_eq!(config.channel_capacity, DEFAULT_CHANNEL_CAPACITY);
    }

    #[test]
    fn test_config_validation_valid() {
        let config = AppConfig {
            server: ServerConfig {
                bind: "127.0.0.1".to_string(),
                port: 8080,
            },
            database: DatabaseConfig {
                driver: DatabaseDriver::Sqlite,
                dsn: "./test.db".to_string(),
                channel_capacity: 1000,
            },
            collector_include: None,
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
            collector_include: None,
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
            collector_include: None,
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

    #[test]
    fn test_database_driver_display() {
        assert_eq!(DatabaseDriver::Sqlite.to_string(), "sqlite");
        assert_eq!(DatabaseDriver::Postgresql.to_string(), "postgresql");
    }

    #[test]
    fn test_connection_url_sqlite_plain_path() {
        let config = DatabaseConfig {
            driver: DatabaseDriver::Sqlite,
            dsn: "data/oculus.db".to_string(),
            channel_capacity: 1000,
        };
        assert_eq!(config.connection_url(), "sqlite:data/oculus.db?mode=rwc");
    }

    #[test]
    fn test_connection_url_sqlite_with_prefix() {
        let config = DatabaseConfig {
            driver: DatabaseDriver::Sqlite,
            dsn: "sqlite:data/oculus.db?mode=rwc".to_string(),
            channel_capacity: 1000,
        };
        // Should not double-prefix
        assert_eq!(config.connection_url(), "sqlite:data/oculus.db?mode=rwc");
    }

    #[test]
    fn test_connection_url_sqlite_with_query() {
        let config = DatabaseConfig {
            driver: DatabaseDriver::Sqlite,
            dsn: "data/oculus.db?mode=rw".to_string(),
            channel_capacity: 1000,
        };
        // Should not add mode=rwc if query already exists
        assert_eq!(config.connection_url(), "sqlite:data/oculus.db?mode=rw");
    }

    #[test]
    fn test_connection_url_postgresql() {
        let config = DatabaseConfig {
            driver: DatabaseDriver::Postgresql,
            dsn: "postgres://user:pass@localhost:5432/oculus".to_string(),
            channel_capacity: 1000,
        };
        // PostgreSQL URLs should pass through as-is
        assert_eq!(
            config.connection_url(),
            "postgres://user:pass@localhost:5432/oculus"
        );
    }
}
