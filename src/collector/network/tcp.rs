//! TCP port probe collector.
//!
//! Measures TCP connection latency to a target address.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::collector::{Collector, CollectorError, Schedule};
use crate::storage::{MetricCategory, MetricSeries, MetricValue, StaticTags, StorageWriter};

/// Default collection interval (30 seconds).
const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Default connection timeout (3 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

/// Latency value indicating probe failure (connection refused, timeout, etc.).
/// Using -1.0 to distinguish from valid 0ms latency.
const FAILURE_LATENCY_MS: f64 = -1.0;

/// Configuration for TCP port probe.
#[derive(Debug, Clone)]
pub struct TcpConfig {
    /// Unique name for this probe instance.
    pub name: String,
    /// Target socket address (host:port).
    pub target_addr: SocketAddr,
    /// Static tags for identity.
    pub static_tags: StaticTags,
    /// Human-readable description.
    pub description: Option<String>,
    /// Execution schedule.
    pub schedule: Schedule,
    /// Connection timeout.
    pub conn_timeout: Duration,
}

impl TcpConfig {
    /// Create a new TCP probe configuration.
    pub fn new(name: impl Into<String>, target: SocketAddr) -> Self {
        Self {
            name: name.into(),
            target_addr: target,
            schedule: Schedule::interval(DEFAULT_INTERVAL),
            conn_timeout: DEFAULT_TIMEOUT,
            static_tags: StaticTags::new(),
            description: None,
        }
    }

    /// Set the execution schedule.
    pub fn with_schedule(mut self, schedule: Schedule) -> Self {
        self.schedule = schedule;
        self
    }

    /// Set the connection timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.conn_timeout = timeout;
        self
    }

    /// Set static tags.
    pub fn with_static_tags(mut self, tags: StaticTags) -> Self {
        self.static_tags = tags;
        self
    }

    /// Set description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
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
        let series_id = MetricSeries::compute_series_id(
            MetricCategory::NetworkTcp,
            &config.name,
            &config.target_addr.to_string(),
            &config.static_tags,
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

    fn schedule(&self) -> &Schedule {
        &self.config.schedule
    }

    fn upsert_metric_series(&self) -> Result<u64, CollectorError> {
        // Create series
        let series = MetricSeries::new(
            MetricCategory::NetworkTcp,
            self.config.name.clone(),
            self.config.target_addr.to_string(),
            self.config.static_tags.clone(),
            self.config.description.clone(),
        );

        self.writer.upsert_metric_series(series)?;
        Ok(self.series_id)
    }

    async fn collect(&self) -> Result<(), CollectorError> {
        let target = self.config.target_addr;
        let probe_timeout = self.config.conn_timeout;

        // Measure connection time
        let start = Instant::now();
        let result = timeout(probe_timeout, TcpStream::connect(target)).await;
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
        let value = MetricValue::new(self.series_id, latency_ms, success, duration_ms)
            .with_tag("schedule", self.config.schedule.to_string());

        self.writer.insert_metric_value(value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBuilder;
    use std::io::ErrorKind;
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    #[test]
    fn test_tcp_config_defaults() {
        let addr: SocketAddr = "127.0.0.1:6379".parse().unwrap();
        let config = TcpConfig::new("redis", addr);

        assert_eq!(config.name, "redis");
        assert_eq!(config.target_addr, addr);
        // Verify default schedule is an interval
        assert!(matches!(config.schedule, Schedule::Interval(d) if d == DEFAULT_INTERVAL));
        assert_eq!(config.conn_timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_tcp_config_builder() {
        let addr: SocketAddr = "127.0.0.1:3306".parse().unwrap();
        let config = TcpConfig::new("mysql", addr)
            .with_schedule(Schedule::interval(Duration::from_secs(60)))
            .with_timeout(Duration::from_secs(10));

        assert!(matches!(&config.schedule, Schedule::Interval(d) if *d == Duration::from_secs(60)));
        assert_eq!(config.conn_timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_tcp_config_cron() {
        let addr: SocketAddr = "127.0.0.1:5432".parse().unwrap();
        // 6-field cron: every 5 minutes at second 0
        let schedule = Schedule::cron("0 */5 * * * *").expect("valid cron");
        let config = TcpConfig::new("postgres", addr).with_schedule(schedule);

        // Verify cron expression is stored correctly
        match &config.schedule {
            Schedule::Cron(expr) => assert_eq!(expr, "0 */5 * * * *"),
            _ => panic!("expected Cron schedule"),
        }
    }

    // =========================================================================
    // Integration tests for TcpCollector
    // =========================================================================

    #[tokio::test]
    async fn test_tcp_collector_success() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tcp_success.db");

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

        let handles = StorageBuilder::new(&db_path).build().unwrap();
        let config = TcpConfig::new("test-success", addr).with_timeout(Duration::from_secs(1));
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

        handles.shutdown().unwrap();
    }

    #[tokio::test]
    async fn test_tcp_collector_connection_refused() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tcp_refused.db");

        // Use a port that is very likely to be unused (no listener)
        let addr: SocketAddr = "127.0.0.1:59999".parse().unwrap();

        let handles = StorageBuilder::new(&db_path).build().unwrap();
        let config = TcpConfig::new("test-refused", addr).with_timeout(Duration::from_millis(500));
        let collector = TcpCollector::new(config, handles.writer.clone());

        // Upsert series first
        collector.upsert_metric_series().unwrap();

        // Run collection - should succeed (records failure metric, doesn't error)
        let result = collector.collect().await;
        assert!(
            result.is_ok(),
            "collect() should succeed even when connection is refused"
        );

        handles.shutdown().unwrap();
    }

    #[tokio::test]
    async fn test_tcp_collector_timeout() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tcp_timeout.db");

        // Use a non-routable IP (10.255.255.1) which will cause connection to hang/timeout
        // This simulates network timeout better than connection refused
        let addr: SocketAddr = "10.255.255.1:80".parse().unwrap();

        let handles = StorageBuilder::new(&db_path).build().unwrap();
        let config = TcpConfig::new("test-timeout", addr).with_timeout(Duration::from_millis(100));
        let collector = TcpCollector::new(config, handles.writer.clone());

        // Upsert series first
        collector.upsert_metric_series().unwrap();

        // Run collection - should succeed (records failure metric, doesn't error)
        let result = collector.collect().await;
        assert!(
            result.is_ok(),
            "collect() should succeed even when connection times out"
        );

        handles.shutdown().unwrap();
    }
}
