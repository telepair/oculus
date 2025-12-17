//! ICMP ping probe collector.
//!
//! Measures ICMP ping latency to a target host.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence};
use tokio::time::timeout;

use crate::collector::traits::{IpValidationError, validate_ip_address};
use crate::collector::{Collector, CollectorError, Schedule};
use crate::storage::{MetricCategory, MetricSeries, MetricValue, StaticTags, StorageWriter};

/// Default collection interval (30 seconds).
const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Default ping timeout (3 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

/// Latency value indicating probe failure (unreachable, timeout, etc.).
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

/// Configuration for ICMP ping probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingConfig {
    /// Unique name for this probe instance.
    pub name: String,
    /// Target host (hostname or IP address).
    pub host: String,
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

impl PingConfig {
    /// Create a new ping probe configuration.
    pub fn new(name: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            host: host.into(),
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
    pub fn static_tags(&self) -> StaticTags {
        self.tags.clone()
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

/// ICMP ping probe collector.
///
/// Measures ICMP ping latency and reports success/failure.
pub struct PingCollector {
    config: PingConfig,
    writer: StorageWriter,
    series_id: u64,
}

impl PingCollector {
    /// Create a new ping collector with the given configuration and writer.
    pub fn new(config: PingConfig, writer: StorageWriter) -> Self {
        let series_id = MetricSeries::compute_series_id(
            MetricCategory::NetworkPing,
            &config.name,
            &config.host,
            &config.static_tags(),
        );

        Self {
            config,
            writer,
            series_id,
        }
    }
}

impl std::fmt::Debug for PingCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PingCollector")
            .field("config", &self.config)
            .field("series_id", &self.series_id)
            .finish_non_exhaustive()
    }
}

/// Resolve hostname to IP address.
async fn resolve_host(host: &str) -> Result<IpAddr, std::io::Error> {
    // First, try to parse as an IP address directly
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip);
    }

    // Otherwise, resolve the hostname using tokio's DNS lookup
    let addrs = tokio::net::lookup_host(format!("{host}:0")).await?;
    addrs
        .into_iter()
        .next()
        .map(|addr| addr.ip())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no addresses found"))
}

#[async_trait::async_trait]
impl Collector for PingCollector {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn category(&self) -> MetricCategory {
        MetricCategory::NetworkPing
    }

    fn schedule(&self) -> Schedule {
        Schedule::Interval(self.config.interval)
    }

    fn upsert_metric_series(&self) -> Result<u64, CollectorError> {
        // Create series
        let series = MetricSeries::new(
            MetricCategory::NetworkPing,
            self.config.name.clone(),
            self.config.host.clone(),
            self.config.static_tags(),
            self.config.description.clone(),
        );

        self.writer.upsert_metric_series(series)?;
        Ok(self.series_id)
    }

    async fn collect(&self) -> Result<(), CollectorError> {
        let probe_timeout = self.config.timeout;

        // Resolve hostname to IP address
        let ip_addr = match resolve_host(&self.config.host).await {
            Ok(ip) => ip,
            Err(e) => {
                tracing::warn!(
                    name = %self.config.name,
                    host = %self.config.host,
                    error = %e,
                    "Failed to resolve hostname"
                );
                // Record failure metric
                let value = MetricValue::new(self.series_id, FAILURE_LATENCY_MS, false)
                    .with_unit("ms")
                    .with_duration_ms(0)
                    .with_tag("error", "dns_resolution_failed")
                    .with_tag("interval", format!("{}s", self.config.interval.as_secs()));
                self.writer.insert_metric_value(value)?;
                return Ok(());
            }
        };

        // Create ICMP client based on IP version
        let client = match ip_addr {
            IpAddr::V4(_) => Client::new(&Config::default()),
            IpAddr::V6(_) => Client::new(&Config::builder().kind(ICMP::V6).build()),
        };

        let client = match client {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    name = %self.config.name,
                    host = %self.config.host,
                    error = %e,
                    "Failed to create ICMP client"
                );
                // Record failure metric
                let value = MetricValue::new(self.series_id, FAILURE_LATENCY_MS, false)
                    .with_unit("ms")
                    .with_duration_ms(0)
                    .with_tag("error", "client_creation_failed")
                    .with_tag("interval", format!("{}s", self.config.interval.as_secs()));
                self.writer.insert_metric_value(value)?;
                return Ok(());
            }
        };

        // Create pinger
        let mut pinger = client.pinger(ip_addr, PingIdentifier(rand::random())).await;
        pinger.timeout(probe_timeout);

        // Measure ping time
        let start = Instant::now();
        let result = timeout(probe_timeout, pinger.ping(PingSequence(0), &[])).await;
        let elapsed = start.elapsed();
        let duration_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;

        let (latency_ms, success) = match result {
            Ok(Ok((_, rtt))) => {
                let ms = rtt.as_secs_f64() * 1000.0;
                tracing::debug!(
                    name = %self.config.name,
                    host = %self.config.host,
                    latency_ms = ms,
                    "Ping probe successful"
                );
                (ms, true)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    name = %self.config.name,
                    host = %self.config.host,
                    error = %e,
                    "Ping probe failed"
                );
                (FAILURE_LATENCY_MS, false)
            }
            Err(_) => {
                tracing::warn!(
                    name = %self.config.name,
                    host = %self.config.host,
                    timeout_ms = probe_timeout.as_millis(),
                    "Ping probe timed out"
                );
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

    #[test]
    fn test_ping_config_defaults() {
        let config = PingConfig::new("google-dns", "8.8.8.8");

        assert_eq!(config.name, "google-dns");
        assert_eq!(config.host, "8.8.8.8");
        // Verify default schedule is an interval
        assert_eq!(config.interval, DEFAULT_INTERVAL);
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_ping_config_builder() {
        let config = PingConfig::new("cloudflare", "1.1.1.1")
            .with_interval(Duration::from_secs(60))
            .with_timeout(Duration::from_secs(10))
            .with_description("Cloudflare DNS");

        assert_eq!(config.interval, Duration::from_secs(60));
        assert_eq!(config.timeout, Duration::from_secs(10));
        assert_eq!(config.description, Some("Cloudflare DNS".to_string()));
    }

    #[test]
    fn test_ping_config_hostname() {
        let config = PingConfig::new("google", "google.com");
        assert_eq!(config.host, "google.com");
    }

    #[tokio::test]
    async fn test_resolve_host_ipv4() {
        let ip = resolve_host("127.0.0.1").await.unwrap();
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[tokio::test]
    async fn test_resolve_host_ipv6() {
        let ip = resolve_host("::1").await.unwrap();
        assert_eq!(ip, IpAddr::V6(std::net::Ipv6Addr::LOCALHOST));
    }
}
