//! Collector configuration structures.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::collector::http::HttpConfig;
use crate::collector::ping::PingConfig;
use crate::collector::tcp::TcpConfig;

use super::validation::ConfigError;

/// Collectors configuration grouped by type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectorsConfig {
    /// TCP port probe collectors.
    #[serde(default)]
    pub tcp: Vec<TcpConfig>,

    /// ICMP ping probe collectors.
    #[serde(default)]
    pub ping: Vec<PingConfig>,

    /// HTTP endpoint probe collectors.
    #[serde(default)]
    pub http: Vec<HttpConfig>,
}

impl CollectorsConfig {
    /// Merge another CollectorsConfig into this one.
    #[must_use]
    pub fn merge(mut self, other: CollectorsConfig) -> Self {
        self.tcp.extend(other.tcp);
        self.ping.extend(other.ping);
        self.http.extend(other.http);
        self
    }

    /// Validate all collector configurations.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut seen_names = HashSet::new();

        // Validate TCP collectors
        for tcp in &self.tcp {
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
            tcp.validate().map_err(|e| {
                ConfigError::ValidationError(format!("tcp collector '{}': {}", tcp.name, e))
            })?;
        }

        // Validate ping collectors
        for ping in &self.ping {
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
            // Ping allows hostnames, so skip strict IP validation
        }

        // Validate HTTP collectors
        for http in &self.http {
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
            // Validate URL
            url::Url::parse(&http.url).map_err(|e| {
                ConfigError::ValidationError(format!(
                    "http collector '{}': invalid URL '{}': {}",
                    http.name, http.url, e
                ))
            })?;
            // Validate interval vs cron
            if http.interval.is_some() && http.cron.is_some() {
                return Err(ConfigError::ValidationError(format!(
                    "http collector '{}': cannot specify both interval and cron",
                    http.name
                )));
            }
        }

        Ok(())
    }

    /// Load collector configurations from all YAML files in a directory.
    pub fn load_from_dir(dir_path: &str) -> Result<Self, ConfigError> {
        let dir = Path::new(dir_path);
        if !dir.exists() {
            return Err(ConfigError::ValidationError(format!(
                "collector_path '{}' does not exist",
                dir_path
            )));
        }
        if !dir.is_dir() {
            return Err(ConfigError::ValidationError(format!(
                "collector_path '{}' is not a directory",
                dir_path
            )));
        }

        let mut merged = Self::default();
        let entries = std::fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "yaml" && ext != "yml" {
                continue;
            }

            tracing::debug!("Loading collector config from: {}", path.display());
            let content = std::fs::read_to_string(&path)?;
            let file_config: Self = serde_yaml::from_str(&content).map_err(|e| {
                ConfigError::ValidationError(format!("failed to parse '{}': {}", path.display(), e))
            })?;

            merged = merged.merge(file_config);
        }

        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collectors_config_merge() {
        let config1 = CollectorsConfig {
            tcp: vec![TcpConfig::new("tcp-1", "127.0.0.1", 6379)],
            ping: vec![],
            http: vec![],
        };

        let config2 = CollectorsConfig {
            tcp: vec![TcpConfig::new("tcp-2", "127.0.0.1", 6380)],
            ping: vec![PingConfig::new("ping-1", "8.8.8.8")],
            http: vec![],
        };

        let merged = config1.merge(config2);
        assert_eq!(merged.tcp.len(), 2);
        assert_eq!(merged.ping.len(), 1);
    }

    #[test]
    fn test_collectors_config_validate_duplicate_names() {
        let config = CollectorsConfig {
            tcp: vec![
                TcpConfig::new("duplicate", "127.0.0.1", 6379),
                TcpConfig::new("duplicate", "127.0.0.1", 6380),
            ],
            ping: vec![],
            http: vec![],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn test_collectors_config_validate_cross_type_duplicate() {
        let config = CollectorsConfig {
            tcp: vec![TcpConfig::new("same-name", "127.0.0.1", 6379)],
            ping: vec![PingConfig::new("same-name", "8.8.8.8")],
            http: vec![],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn test_collectors_config_validate_empty_name() {
        let mut tcp = TcpConfig::new("", "127.0.0.1", 6379);
        tcp.name = "".to_string();

        let config = CollectorsConfig {
            tcp: vec![tcp],
            ping: vec![],
            http: vec![],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_collectors_config_validate_invalid_http_url() {
        use crate::collector::http::HttpConfig;

        let mut http = HttpConfig::new("test", "https://valid.url");
        http.url = "not-a-valid-url".to_string();

        let config = CollectorsConfig {
            tcp: vec![],
            ping: vec![],
            http: vec![http],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid URL"));
    }

    #[test]
    fn test_collectors_config_validate_http_interval_cron_conflict() {
        use crate::collector::http::HttpConfig;
        use std::time::Duration;

        let mut http = HttpConfig::new("test", "https://valid.url");
        http.interval = Some(Duration::from_secs(30));
        http.cron = Some("0 * * * * *".to_string());

        let config = CollectorsConfig {
            tcp: vec![],
            ping: vec![],
            http: vec![http],
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot specify both interval and cron")
        );
    }

    #[test]
    fn test_tcp_config_serde_roundtrip() {
        let yaml = r#"
name: test-tcp
host: 127.0.0.1
port: 6379
enabled: false
group: production
interval: 10s
timeout: 2s
tags:
  env: test
description: Test TCP probe
"#;

        let config: TcpConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "test-tcp");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 6379);
        assert!(!config.enabled);
        assert_eq!(config.group, "production");
        assert_eq!(config.interval.as_secs(), 10);
        assert_eq!(config.timeout.as_secs(), 2);
        assert_eq!(config.tags.get("env"), Some(&"test".to_string()));
        assert_eq!(config.description, Some("Test TCP probe".to_string()));
    }

    #[test]
    fn test_tcp_config_serde_defaults() {
        let yaml = r#"
name: minimal
host: 127.0.0.1
port: 80
"#;

        let config: TcpConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.enabled); // default: true
        assert_eq!(config.group, "default"); // default: "default"
        assert_eq!(config.interval.as_secs(), 30); // default: 30s
        assert_eq!(config.timeout.as_secs(), 3); // default: 3s
    }

    #[test]
    fn test_http_config_serde_roundtrip() {
        use crate::collector::http::HttpConfig;

        let yaml = r#"
name: test-http
url: https://api.example.com/health
enabled: true
group: production
method: POST
expected_status: 201
interval: 60s
timeout: 15s
headers:
  Authorization: Bearer token
"#;

        let config: HttpConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "test-http");
        assert_eq!(config.url, "https://api.example.com/health");
        assert!(config.enabled);
        assert_eq!(config.group, "production");
        assert_eq!(config.expected_status, 201);
        assert_eq!(config.interval, Some(std::time::Duration::from_secs(60)));
        assert_eq!(config.timeout.as_secs(), 15);
        assert_eq!(
            config.headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
    }
}
