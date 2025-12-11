//! TCP port probe collector.
//!
//! Measures TCP connection latency to a target address.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::Metric;
use crate::collector::{Collector, CollectorConfig, CollectorError, Schedule};
use crate::storage::StorageWriter;

/// Default collection interval (30 seconds).
const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Default connection timeout (3 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

/// Category for metrics.
const CATEGORY_TCP_PROBE: &str = "network.tcp.probe";

/// Configuration for TCP port probe.
#[derive(Debug, Clone)]
pub struct TcpConfig {
    /// Unique name for this probe instance.
    name: String,
    /// Target socket address (host:port).
    target: SocketAddr,
    /// Execution schedule.
    schedule: Schedule,
    /// Connection timeout.
    conn_timeout: Duration,
}

impl TcpConfig {
    /// Create a new TCP probe configuration.
    ///
    /// # Arguments
    /// * `name` - Unique identifier for this probe
    /// * `target` - Target socket address (e.g., "127.0.0.1:6379")
    pub fn new(name: impl Into<String>, target: SocketAddr) -> Self {
        Self {
            name: name.into(),
            target,
            schedule: Schedule::interval(DEFAULT_INTERVAL),
            conn_timeout: DEFAULT_TIMEOUT,
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

    /// Get the target address.
    pub fn target(&self) -> SocketAddr {
        self.target
    }
}

impl CollectorConfig for TcpConfig {
    fn name(&self) -> &str {
        &self.name
    }

    fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    fn timeout(&self) -> Duration {
        self.conn_timeout
    }
}

/// TCP port probe collector.
///
/// Measures TCP connection latency and reports success/failure.
///
/// # Metrics
///
/// Produces metrics with:
/// - **symbol**: `net.tcp.<host>:<port>`
/// - **value**: latency in milliseconds, or -1.0 on failure
/// - **tags**: `{"target": "<addr>", "probe": "tcp", "success": true/false}`
pub struct TcpCollector {
    config: TcpConfig,
    writer: StorageWriter,
}

#[async_trait::async_trait]
impl Collector for TcpCollector {
    type Config = TcpConfig;

    fn new(config: Self::Config, writer: StorageWriter) -> Self {
        Self { config, writer }
    }

    fn category(&self) -> &str {
        CATEGORY_TCP_PROBE
    }

    fn config(&self) -> &Self::Config {
        &self.config
    }

    async fn collect(&self) -> Result<(), CollectorError> {
        let target = self.config.target;
        let probe_timeout = self.config.conn_timeout;

        // Measure connection time
        let start = Instant::now();
        let result = timeout(probe_timeout, TcpStream::connect(target)).await;
        let elapsed = start.elapsed();

        let (latency_ms, success) = match result {
            Ok(Ok(_stream)) => {
                // Connection successful
                let ms = elapsed.as_secs_f64() * 1000.0;
                tracing::debug!(
                    name = %self.config.name,
                    target = %target,
                    latency_ms = ms,
                    "TCP probe successful"
                );
                (Some(ms), true)
            }
            Ok(Err(e)) => {
                // Connection failed (refused, unreachable, etc.)
                tracing::warn!(
                    name = %self.config.name,
                    target = %target,
                    error = %e,
                    "TCP probe failed"
                );
                (None, false)
            }
            Err(_) => {
                // Timeout
                tracing::warn!(
                    name = %self.config.name,
                    target = %target,
                    timeout_ms = probe_timeout.as_millis(),
                    "TCP probe timed out"
                );
                (None, false)
            }
        };

        // Build metric
        let symbol = format!("net.tcp.{}:{}", target.ip(), target.port());
        let tags = serde_json::json!({
            "name": self.config.name,
            "target": target.to_string(),
            "schedule": self.config.schedule.to_string(),
            "success": success,
        });

        let metric = Metric {
            ts: Utc::now(),
            category: CATEGORY_TCP_PROBE.to_string(),
            symbol,
            value: latency_ms.unwrap_or(-1.0),
            tags: Some(tags),
            success,
            duration_ms: elapsed.as_millis() as i64,
        };

        self.writer.insert_metric(metric)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBuilder;
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    #[test]
    fn test_tcp_config_defaults() {
        let addr: SocketAddr = "127.0.0.1:6379".parse().unwrap();
        let config = TcpConfig::new("redis", addr);

        assert_eq!(config.name(), "redis");
        assert_eq!(config.target(), addr);
        // Verify default schedule is an interval
        assert!(matches!(config.schedule(), Schedule::Interval(d) if *d == DEFAULT_INTERVAL));
        assert_eq!(config.timeout(), DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_tcp_config_builder() {
        let addr: SocketAddr = "127.0.0.1:3306".parse().unwrap();
        let config = TcpConfig::new("mysql", addr)
            .with_schedule(Schedule::interval(Duration::from_secs(60)))
            .with_timeout(Duration::from_secs(10));

        assert!(
            matches!(config.schedule(), Schedule::Interval(d) if *d == Duration::from_secs(60))
        );
        assert_eq!(config.timeout(), Duration::from_secs(10));
    }

    #[test]
    fn test_tcp_config_cron() {
        let addr: SocketAddr = "127.0.0.1:5432".parse().unwrap();
        // 6-field cron: every 5 minutes at second 0
        let schedule = Schedule::cron("0 */5 * * * *").expect("valid cron");
        let config = TcpConfig::new("postgres", addr).with_schedule(schedule);

        // Verify cron expression is stored correctly
        match config.schedule() {
            Schedule::Cron(expr) => assert_eq!(expr, "0 */5 * * * *"),
            _ => panic!("expected Cron schedule"),
        }
    }

    // =========================================================================
    // Integration tests for TcpCollector::collect()
    // =========================================================================

    #[tokio::test]
    async fn test_tcp_collector_success() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tcp_success.db");

        // Start a mock TCP listener on a random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
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

        // Run collection - should succeed (records failure metric, doesn't error)
        let result = collector.collect().await;
        assert!(
            result.is_ok(),
            "collect() should succeed even when connection times out"
        );

        handles.shutdown().unwrap();
    }
}
