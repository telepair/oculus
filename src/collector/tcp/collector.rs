//! TCP port probe collector.
//!
//! Measures TCP connection latency to a target address.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::collector::traits::{IpValidationError, validate_ip_address};
use crate::collector::{Collector, CollectorError, Schedule};
use crate::storage::{MetricCategory, MetricSeries, MetricValue, StaticTags, StorageWriter};

/// Default collection interval (30 seconds).
const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Default connection timeout (3 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

/// Latency value indicating probe failure (connection refused, timeout, etc.).
/// Using -1.0 to distinguish from valid 0ms latency.
const FAILURE_LATENCY_MS: f64 = -1.0;

fn default_enabled() -> bool {
    true
}

fn default_group() -> String {
    "default".to_string()
}

fn default_interval() -> Duration {
    DEFAULT_INTERVAL
}

fn default_timeout() -> Duration {
    DEFAULT_TIMEOUT
}

/// Configuration for TCP port probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpConfig {
    /// Unique name for this probe instance.
    pub name: String,
    /// Target host (IP address).
    pub host: String,
    /// Target port.
    pub port: u16,
    /// Enable this collector (default: true).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Collector group for organization (default: "default").
    #[serde(default = "default_group")]
    pub group: String,
    /// Collection interval (default: 30s).
    #[serde(default = "default_interval", with = "humantime_serde")]
    pub interval: Duration,
    /// Probe timeout (default: 3s).
    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub timeout: Duration,
    /// Static tags for identity.
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
}

impl TcpConfig {
    /// Create a new TCP probe configuration.
    pub fn new(name: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            host: host.into(),
            port,
            enabled: true,
            group: "default".to_string(),
            interval: DEFAULT_INTERVAL,
            timeout: DEFAULT_TIMEOUT,
            tags: BTreeMap::new(),
            description: None,
        }
    }

    /// Validate the configuration.
    ///
    /// Returns an error if the host is not a valid IP address.
    pub fn validate(&self) -> Result<(), IpValidationError> {
        validate_ip_address(&self.host)?;
        Ok(())
    }

    /// Get static tags as StaticTags type.
    pub fn static_tags(&self) -> &StaticTags {
        &self.tags
    }

    /// Set the collection interval.
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Set the probe timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set static tags.
    pub fn with_static_tags(mut self, tags: StaticTags) -> Self {
        self.tags = tags;
        self
    }

    /// Set description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set enabled.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set group.
    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }
}

/// TCP port probe collector.
///
/// Measures TCP connection latency and reports success/failure.
pub struct TcpCollector {
    config: TcpConfig,
    writer: StorageWriter,
    series_id: u64,
}

impl TcpCollector {
    /// Create a new TCP collector with the given configuration and writer.
    pub fn new(config: TcpConfig, writer: StorageWriter) -> Self {
        let target = format!("{}:{}", config.host, config.port);
        let series_id = MetricSeries::compute_series_id(
            MetricCategory::NetworkTcp,
            &config.name,
            &target,
            config.static_tags(),
        );

        Self {
            config,
            writer,
            series_id,
        }
    }
}

impl std::fmt::Debug for TcpCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpCollector")
            .field("config", &self.config)
            .field("series_id", &self.series_id)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Collector for TcpCollector {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn category(&self) -> MetricCategory {
        MetricCategory::NetworkTcp
    }

    fn schedule(&self) -> Schedule {
        Schedule::Interval(self.config.interval)
    }

    fn upsert_metric_series(&self) -> Result<u64, CollectorError> {
        // Create series
        let target = format!("{}:{}", self.config.host, self.config.port);
        let series = MetricSeries::new(
            MetricCategory::NetworkTcp,
            self.config.name.clone(),
            target,
            self.config.static_tags().clone(),
            self.config.description.clone(),
        );

        self.writer.upsert_metric_series(series)?;
        Ok(self.series_id)
    }

    async fn collect(&self) -> Result<(), CollectorError> {
        let target = format!("{}:{}", self.config.host, self.config.port);
        let probe_timeout = self.config.timeout;

        // Measure connection time
        let start = Instant::now();
        let result = timeout(probe_timeout, TcpStream::connect(&target)).await;
        let elapsed = start.elapsed();
        let duration_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;

        let (latency_ms, success) = match result {
            Ok(Ok(_stream)) => {
                let ms = elapsed.as_secs_f64() * 1000.0;
                tracing::debug!(name = %self.config.name, target = %target, latency_ms = ms, "TCP probe successful");
                (ms, true)
            }
            Ok(Err(e)) => {
                tracing::warn!(name = %self.config.name, target = %target, error = %e, "TCP probe failed");
                (FAILURE_LATENCY_MS, false)
            }
            Err(_) => {
                tracing::warn!(name = %self.config.name, target = %target, timeout_ms = probe_timeout.as_millis(), "TCP probe timed out");
                (FAILURE_LATENCY_MS, false)
            }
        };

        // Create value with dynamic tags
        let value = MetricValue::new(self.series_id, latency_ms, success)
            .with_unit("ms")
            .with_duration_ms(duration_ms)
            .with_tag("interval", format!("{}s", self.config.interval.as_secs()));

        self.writer.insert_metric_value(value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBuilder;
    use std::io::ErrorKind;
    use tokio::net::TcpListener;

    #[test]
    fn test_tcp_config_defaults() {
        let config = TcpConfig::new("redis", "127.0.0.1", 6379);

        assert_eq!(config.name, "redis");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 6379);
        // Verify default schedule is an interval
        assert_eq!(config.interval, DEFAULT_INTERVAL);
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_tcp_config_builder() {
        let config = TcpConfig::new("mysql", "127.0.0.1", 3306)
            .with_interval(Duration::from_secs(60))
            .with_timeout(Duration::from_secs(10));

        assert_eq!(config.interval, Duration::from_secs(60));
        assert_eq!(config.timeout, Duration::from_secs(10));
    }

    // =========================================================================
    // Integration tests for TcpCollector
    // =========================================================================

    #[tokio::test]
    async fn test_tcp_collector_success() {
        // Start a mock TCP listener on a random port
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                // Some sandboxed environments disallow binding; skip the test.
                return;
            }
            Err(e) => panic!("Failed to bind test listener: {e}"),
        };
        let addr = listener.local_addr().unwrap();

        // Accept connections in background (just consume them)
        tokio::spawn(async move {
            loop {
                let _ = listener.accept().await;
            }
        });

        let handles = StorageBuilder::new("sqlite::memory:")
            .build()
            .await
            .unwrap();
        let config = TcpConfig::new("test-success", addr.ip().to_string(), addr.port())
            .with_timeout(Duration::from_secs(1));
        let collector = TcpCollector::new(config, handles.writer.clone());

        // Upsert series first
        let series_id = collector.upsert_metric_series().unwrap();
        assert!(series_id > 0);

        // Run collection - should succeed without errors
        let result = collector.collect().await;
        assert!(
            result.is_ok(),
            "collect() should succeed for reachable target"
        );

        handles.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_tcp_collector_connection_refused() {
        // Use a port that is very likely to be unused (no listener)

        let handles = StorageBuilder::new("sqlite::memory:")
            .build()
            .await
            .unwrap();
        let config = TcpConfig::new("test-refused", "127.0.0.1", 59999)
            .with_timeout(Duration::from_millis(500));
        let collector = TcpCollector::new(config, handles.writer.clone());

        // Upsert series first
        collector.upsert_metric_series().unwrap();

        // Run collection - should succeed (records failure metric, doesn't error)
        let result = collector.collect().await;
        assert!(
            result.is_ok(),
            "collect() should succeed even when connection is refused"
        );

        handles.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_tcp_collector_timeout() {
        // Use a non-routable IP (10.255.255.1) which will cause connection to hang/timeout
        // This simulates network timeout better than connection refused

        let handles = StorageBuilder::new("sqlite::memory:")
            .build()
            .await
            .unwrap();
        let config = TcpConfig::new("test-timeout", "10.255.255.1", 80)
            .with_timeout(Duration::from_millis(100));
        let collector = TcpCollector::new(config, handles.writer.clone());

        // Upsert series first
        collector.upsert_metric_series().unwrap();

        // Run collection - should succeed (records failure metric, doesn't error)
        let result = collector.collect().await;
        assert!(
            result.is_ok(),
            "collect() should succeed even when connection times out"
        );

        handles.shutdown().await.unwrap();
    }
}
