//! Configuration module for Oculus application.
//!
//! Provides YAML-based configuration loading and validation for:
//! - Server settings (port, bind address)
//! - Database settings (path, pool size, channel capacity)
//! - Collector definitions (type, name, target, schedule, timeout)

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

use crate::collector::http::{HttpConfig, HttpMethod};
use crate::collector::ping::PingConfig;
use crate::collector::tcp::TcpConfig;
use crate::collector::traits::validate_ip_address;

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

    /// Collector configurations grouped by type.
    #[serde(default)]
    pub collectors: CollectorsConfig,
}

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

fn default_pool_size() -> u32 {
    DEFAULT_POOL_SIZE
}

fn default_channel_capacity() -> usize {
    DEFAULT_CHANNEL_CAPACITY
}

fn default_checkpoint_interval() -> String {
    "5s".to_string()
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_expected_status() -> u16 {
    200
}

// =============================================================================
// Collector Configurations
// =============================================================================

/// Collectors configuration grouped by type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectorsConfig {
    /// TCP port probe collectors.
    #[serde(default)]
    pub tcp: Vec<TcpCollectorConfig>,

    /// ICMP ping probe collectors.
    #[serde(default)]
    pub ping: Vec<PingCollectorConfig>,

    /// HTTP endpoint probe collectors.
    #[serde(default)]
    pub http: Vec<HttpCollectorConfig>,
}

/// TCP collector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpCollectorConfig {
    /// Unique collector name.
    pub name: String,

    /// Target host (IP address).
    pub host: String,

    /// Target port.
    pub port: u16,

    /// Collection interval (e.g., "30s", "1m").
    #[serde(default)]
    pub interval: Option<String>,

    /// Probe timeout (e.g., "5s").
    pub timeout: String,

    /// Static tags for this collector.
    #[serde(default)]
    pub tags: BTreeMap<String, String>,

    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
}

impl TcpCollectorConfig {
    /// Convert to TcpConfig.
    pub fn to_tcp_config(&self) -> Result<TcpConfig, ConfigError> {
        // Validate host is a valid IP address
        validate_ip_address(&self.host).map_err(|e| {
            ConfigError::ValidationError(format!("tcp collector '{}': {}", self.name, e))
        })?;

        let (timeout, interval) = parse_collector_timing(
            &self.timeout,
            &self.interval,
            &format!("tcp collector '{}'", self.name),
        )?;

        let mut config = TcpConfig::new(&self.name, &self.host, self.port)
            .with_timeout(timeout)
            .with_interval(interval)
            .with_static_tags(self.tags.clone());

        if let Some(desc) = &self.description {
            config = config.with_description(desc);
        }

        Ok(config)
    }
}

/// Ping collector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingCollectorConfig {
    /// Unique collector name.
    pub name: String,

    /// Target host (IP address or hostname).
    pub host: String,

    /// Collection interval (e.g., "30s", "1m").
    #[serde(default)]
    pub interval: Option<String>,

    /// Probe timeout (e.g., "3s").
    pub timeout: String,

    /// Static tags for this collector.
    #[serde(default)]
    pub tags: BTreeMap<String, String>,

    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
}

impl PingCollectorConfig {
    /// Convert to PingConfig.
    pub fn to_ping_config(&self) -> Result<PingConfig, ConfigError> {
        let (timeout, interval) = parse_collector_timing(
            &self.timeout,
            &self.interval,
            &format!("ping collector '{}'", self.name),
        )?;

        let mut config = PingConfig::new(&self.name, &self.host)
            .with_timeout(timeout)
            .with_interval(interval)
            .with_static_tags(self.tags.clone());

        if let Some(desc) = &self.description {
            config = config.with_description(desc);
        }

        Ok(config)
    }
}

/// HTTP collector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpCollectorConfig {
    /// Unique collector name.
    pub name: String,

    /// Target URL (HTTP or HTTPS).
    pub url: String,

    /// HTTP method (GET, POST, HEAD, etc.).
    #[serde(default = "default_http_method")]
    pub method: String,

    /// Expected HTTP status code (default: 200).
    #[serde(default = "default_expected_status")]
    pub expected_status: u16,

    /// Collection interval (e.g., "30s", "1m").
    #[serde(default)]
    pub interval: Option<String>,

    /// Request timeout (e.g., "10s").
    pub timeout: String,

    /// Static tags for this collector.
    #[serde(default)]
    pub tags: BTreeMap<String, String>,

    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
}

impl HttpCollectorConfig {
    /// Convert to HttpConfig.
    pub fn to_http_config(&self) -> Result<HttpConfig, ConfigError> {
        // Validate URL format
        url::Url::parse(&self.url).map_err(|e| {
            ConfigError::ValidationError(format!(
                "http collector '{}': invalid URL '{}': {}",
                self.name, self.url, e
            ))
        })?;

        let (timeout, interval) = parse_collector_timing(
            &self.timeout,
            &self.interval,
            &format!("http collector '{}'", self.name),
        )?;

        let method: HttpMethod = self.method.parse().map_err(|()| {
            ConfigError::ValidationError(format!(
                "http collector '{}': invalid method '{}', expected GET/POST/HEAD/PUT/DELETE/OPTIONS/PATCH",
                self.name, self.method
            ))
        })?;

        let mut config = HttpConfig::new(&self.name, &self.url)
            .with_method(method)
            .with_expected_status(self.expected_status)
            .with_timeout(timeout)
            .with_interval(interval)
            .with_static_tags(self.tags.clone());

        if let Some(desc) = &self.description {
            config = config.with_description(desc);
        }

        Ok(config)
    }
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

        // Check for duplicate collector names across all types
        let mut seen_names = HashSet::new();

        // Validate TCP collectors
        for tcp in &self.collectors.tcp {
            if tcp.name.is_empty() {
                return Err(ConfigError::ValidationError(
                    "tcp collector name cannot be empty".to_string(),
                ));
            }
            if !seen_names.insert(&tcp.name) {
                return Err(ConfigError::ValidationError(format!(
                    "duplicate collector name: '{}'",
                    tcp.name
                )));
            }
            // Validate by attempting conversion (validates host, timeout, interval)
            tcp.to_tcp_config()?;
        }

        // Validate ping collectors
        for ping in &self.collectors.ping {
            if ping.name.is_empty() {
                return Err(ConfigError::ValidationError(
                    "ping collector name cannot be empty".to_string(),
                ));
            }
            if !seen_names.insert(&ping.name) {
                return Err(ConfigError::ValidationError(format!(
                    "duplicate collector name: '{}'",
                    ping.name
                )));
            }
            ping.to_ping_config()?;
        }

        // Validate HTTP collectors
        for http in &self.collectors.http {
            if http.name.is_empty() {
                return Err(ConfigError::ValidationError(
                    "http collector name cannot be empty".to_string(),
                ));
            }
            if !seen_names.insert(&http.name) {
                return Err(ConfigError::ValidationError(format!(
                    "duplicate collector name: '{}'",
                    http.name
                )));
            }
            http.to_http_config()?;
        }

        Ok(())
    }
}

/// Parse duration string using humantime.
///
/// Supports various formats: `30s`, `1m`, `5m30s`, `1h`, `2h30m`, `1d`, `100ms`, etc.
///
/// # Examples
///
/// ```
/// use oculus::config::parse_duration;
///
/// assert_eq!(parse_duration("30s").unwrap().as_secs(), 30);
/// assert_eq!(parse_duration("1m").unwrap().as_secs(), 60);
/// assert_eq!(parse_duration("2h").unwrap().as_secs(), 7200);
/// assert_eq!(parse_duration("1h30m").unwrap().as_secs(), 5400);
/// ```
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("duration string is empty".to_string());
    }
    humantime::parse_duration(s).map_err(|e| e.to_string())
}

/// Parse timeout and interval for a collector configuration.
///
/// Returns `(timeout, interval)` durations.
fn parse_collector_timing(
    timeout: &str,
    interval: &Option<String>,
    collector_name: &str,
) -> Result<(Duration, Duration), ConfigError> {
    let timeout = parse_duration(timeout).map_err(|e| {
        ConfigError::ValidationError(format!("{}: invalid timeout: {}", collector_name, e))
    })?;

    let interval = match interval {
        Some(i) => parse_duration(i).map_err(|e| {
            ConfigError::ValidationError(format!("{}: invalid interval: {}", collector_name, e))
        })?,
        None => DEFAULT_INTERVAL,
    };

    Ok((timeout, interval))
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
            collectors: CollectorsConfig {
                tcp: vec![TcpCollectorConfig {
                    name: "test-probe".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: 6379,
                    interval: Some("30s".to_string()),
                    timeout: "5s".to_string(),
                    tags: BTreeMap::new(),
                    description: None,
                }],
                ping: vec![],
                http: vec![],
            },
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
            collectors: CollectorsConfig::default(),
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
            collectors: CollectorsConfig::default(),
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
            collectors: CollectorsConfig {
                tcp: vec![TcpCollectorConfig {
                    name: "test".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: 6379,
                    interval: Some("invalid".to_string()),
                    timeout: "5s".to_string(),
                    tags: BTreeMap::new(),
                    description: None,
                }],
                ping: vec![],
                http: vec![],
            },
        };

        assert!(config.validate().is_err());
    }

    // =========================================================================
    // TcpCollectorConfig tests
    // =========================================================================

    #[test]
    fn test_tcp_config_valid() {
        let entry = TcpCollectorConfig {
            name: "redis".to_string(),
            host: "127.0.0.1".to_string(),
            port: 6379,
            interval: Some("30s".to_string()),
            timeout: "5s".to_string(),
            tags: BTreeMap::new(),
            description: None,
        };

        let config = entry.to_tcp_config().unwrap();
        assert_eq!(config.name, "redis");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 6379);
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.interval, Duration::from_secs(30));
    }

    #[test]
    fn test_tcp_config_invalid_host() {
        let entry = TcpCollectorConfig {
            name: "test".to_string(),
            host: "invalid-host".to_string(),
            port: 6379,
            interval: Some("30s".to_string()),
            timeout: "5s".to_string(),
            tags: BTreeMap::new(),
            description: None,
        };

        let result = entry.to_tcp_config();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid IP address")
        );
    }

    // =========================================================================
    // PingCollectorConfig tests
    // =========================================================================

    #[test]
    fn test_ping_config_valid() {
        let entry = PingCollectorConfig {
            name: "google-dns".to_string(),
            host: "8.8.8.8".to_string(),
            interval: Some("60s".to_string()),
            timeout: "3s".to_string(),
            tags: BTreeMap::new(),
            description: None,
        };

        let config = entry.to_ping_config().unwrap();
        assert_eq!(config.name, "google-dns");
        assert_eq!(config.host, "8.8.8.8");
        assert_eq!(config.timeout, Duration::from_secs(3));
        assert_eq!(config.interval, Duration::from_secs(60));
    }

    #[test]
    fn test_ping_config_hostname() {
        let entry = PingCollectorConfig {
            name: "google".to_string(),
            host: "google.com".to_string(),
            interval: Some("30s".to_string()),
            timeout: "5s".to_string(),
            tags: BTreeMap::new(),
            description: None,
        };

        let config = entry.to_ping_config().unwrap();
        assert_eq!(config.name, "google");
        assert_eq!(config.host, "google.com");
    }

    // =========================================================================
    // HttpCollectorConfig tests
    // =========================================================================

    #[test]
    fn test_http_config_valid() {
        let entry = HttpCollectorConfig {
            name: "google-health".to_string(),
            url: "https://www.google.com".to_string(),
            method: "GET".to_string(),
            expected_status: 200,
            interval: Some("30s".to_string()),
            timeout: "10s".to_string(),
            tags: BTreeMap::new(),
            description: None,
        };

        let config = entry.to_http_config().unwrap();
        assert_eq!(config.name, "google-health");
        assert_eq!(config.url, "https://www.google.com");
        assert_eq!(config.timeout, Duration::from_secs(10));
        assert_eq!(config.interval, Duration::from_secs(30));
        assert_eq!(config.expected_status, 200);
    }

    #[test]
    fn test_http_config_custom_method() {
        use crate::collector::http::HttpMethod;

        let entry = HttpCollectorConfig {
            name: "api-create".to_string(),
            url: "https://api.example.com/create".to_string(),
            method: "POST".to_string(),
            expected_status: 201,
            interval: Some("60s".to_string()),
            timeout: "5s".to_string(),
            tags: BTreeMap::new(),
            description: None,
        };

        let config = entry.to_http_config().unwrap();
        assert_eq!(config.method, HttpMethod::Post);
        assert_eq!(config.expected_status, 201);
    }

    #[test]
    fn test_http_config_invalid_method() {
        let entry = HttpCollectorConfig {
            name: "test".to_string(),
            url: "https://example.com".to_string(),
            method: "INVALID".to_string(),
            expected_status: 200,
            interval: Some("30s".to_string()),
            timeout: "5s".to_string(),
            tags: BTreeMap::new(),
            description: None,
        };

        let result = entry.to_http_config();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid method"));
    }

    // =========================================================================
    // New validation tests
    // =========================================================================

    #[test]
    fn test_parse_duration_extended_formats() {
        // humantime supports more formats
        assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("2h 30m").unwrap(), Duration::from_secs(9000));
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
    fn test_config_validation_duplicate_collector_names() {
        let config = AppConfig {
            server: ServerConfig::default(),
            database: DatabaseConfig::default(),
            collectors: CollectorsConfig {
                tcp: vec![TcpCollectorConfig {
                    name: "duplicate".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: 6379,
                    interval: None,
                    timeout: "5s".to_string(),
                    tags: BTreeMap::new(),
                    description: None,
                }],
                ping: vec![PingCollectorConfig {
                    name: "duplicate".to_string(), // Same name as TCP collector
                    host: "8.8.8.8".to_string(),
                    interval: None,
                    timeout: "3s".to_string(),
                    tags: BTreeMap::new(),
                    description: None,
                }],
                http: vec![],
            },
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("duplicate collector name")
        );
    }

    #[test]
    fn test_http_config_invalid_url() {
        let entry = HttpCollectorConfig {
            name: "bad-url".to_string(),
            url: "not-a-valid-url".to_string(),
            method: "GET".to_string(),
            expected_status: 200,
            interval: None,
            timeout: "5s".to_string(),
            tags: BTreeMap::new(),
            description: None,
        };

        let result = entry.to_http_config();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid URL"));
    }

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
}
