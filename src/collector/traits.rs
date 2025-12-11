//! Core collector traits and types.

use std::time::Duration;

use crate::{StorageError, StorageWriter};
use thiserror::Error;

/// Minimum allowed interval (1 second).
pub const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Errors that can occur during collection.
#[derive(Debug, Error)]
pub enum CollectorError {
    /// Network I/O error.
    #[error("network error: {0}")]
    Network(#[from] std::io::Error),

    /// Timeout elapsed.
    #[error("timeout elapsed")]
    Timeout,

    /// Failed to send metric to storage.
    #[error("failed to send metric: {0}")]
    Storage(#[from] StorageError),

    /// Configuration error.
    #[error("config error: {0}")]
    Config(String),

    /// Scheduler error.
    #[error("scheduler error: {0}")]
    Scheduler(String),
}

/// Schedule for collector execution.
///
/// Supports both fixed interval and cron-based scheduling.
/// Validation of cron expressions is deferred to scheduler initialization.
#[derive(Debug, Clone)]
pub enum Schedule {
    /// Fixed interval between collections.
    ///
    /// Interval is clamped to a minimum of 1 second.
    Interval(Duration),

    /// Cron expression for scheduled execution.
    ///
    /// Uses standard cron syntax: `sec min hour day month weekday` (6-field).
    /// Example: `"0 */5 * * * *"` = every 5 minutes at second 0
    Cron(String),
}

impl Schedule {
    /// Create an interval schedule.
    ///
    /// Interval is clamped to a minimum of 1 second.
    pub fn interval(duration: Duration) -> Self {
        if duration < MIN_INTERVAL {
            tracing::warn!(min_interval = ?MIN_INTERVAL,
                "Interval duration is less than minimum allowed. Using minimum duration."
            );
            Self::Interval(MIN_INTERVAL)
        } else {
            Self::Interval(duration)
        }
    }

    /// Create a cron schedule with immediate validation.
    ///
    /// # Errors
    /// Returns `CollectorError::Config` if the cron expression is invalid.
    pub fn cron(expr: impl AsRef<str>) -> Result<Self, CollectorError> {
        use std::str::FromStr;

        // Validate by parsing the cron expression directly (lightweight)
        let test_expr = expr.as_ref();
        cron::Schedule::from_str(test_expr)
            .map_err(|e| CollectorError::Config(format!("invalid cron expression: {e}")))?;

        Ok(Self::Cron(test_expr.to_string()))
    }
}

impl std::fmt::Display for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Interval(d) => write!(f, "every {:?}", d),
            Self::Cron(expr) => write!(f, "cron: {}", expr),
        }
    }
}

/// Configuration trait for collectors.
///
/// Implement this trait to define collector-specific settings.
pub trait CollectorConfig: Send + Sync + 'static {
    /// Unique identifier for this collector instance.
    fn name(&self) -> &str;

    /// Execution schedule (interval or cron).
    fn schedule(&self) -> &Schedule;

    /// Timeout for each probe operation.
    fn timeout(&self) -> Duration;
}

/// Core collector trait for implementing data collectors.
///
/// Collectors are async and run in scheduled jobs. They hold writers internally
/// and perform data collection/submission in `collect()`.
///
/// # Error Handling Philosophy
///
/// The `collect()` method distinguishes between **probe failures** and **collector errors**:
///
/// - **Probe failures** (target unreachable, timeout, connection refused): These are valid
///   observation results and should be recorded as metrics with `success: false`. The
///   `collect()` method should return `Ok(())` in these cases.
///
/// - **Collector errors** (configuration issues, storage write failures): These indicate
///   the collector itself cannot function and require intervention. The `collect()`
///   method should return `Err(CollectorError)` in these cases.
///
/// This design ensures continuous monitoring data even when targets are down, while
/// surfacing real infrastructure problems via error propagation.
#[async_trait::async_trait]
pub trait Collector: Send + Sync + 'static {
    /// Associated configuration type.
    type Config: CollectorConfig;

    /// Create a new collector with configuration and writers.
    fn new(config: Self::Config, writer: StorageWriter) -> Self;

    /// Category for metrics (e.g., "network", "crypto").
    fn category(&self) -> &str;

    /// Get the collector's configuration.
    fn config(&self) -> &Self::Config;

    /// Perform one collection cycle.
    ///
    /// # Behavior
    ///
    /// 1. Probe the target and measure duration
    /// 2. Create metric(s) with `success: true/false` based on probe result
    /// 3. Record `duration_ms` for performance tracking
    /// 4. Send metrics/events via internal writers
    ///
    /// # Returns
    ///
    /// - `Ok(())`: Collection completed (probe succeeded OR failed - both are valid data points)
    /// - `Err(CollectorError::Storage)`: Failed to write to storage (infrastructure problem)
    /// - `Err(CollectorError::Config)`: Invalid configuration (requires manual fix)
    async fn collect(&self) -> Result<(), CollectorError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_interval_minimum() {
        let schedule = Schedule::interval(Duration::from_millis(100));
        match schedule {
            Schedule::Interval(d) => assert_eq!(d, MIN_INTERVAL),
            _ => panic!("expected Interval"),
        }
    }

    #[test]
    fn test_schedule_interval_valid() {
        let schedule = Schedule::interval(Duration::from_secs(30));
        match schedule {
            Schedule::Interval(d) => assert_eq!(d, Duration::from_secs(30)),
            _ => panic!("expected Interval"),
        }
    }

    #[test]
    fn test_schedule_cron_valid() {
        let schedule = Schedule::cron("0 */5 * * * *").unwrap();
        match schedule {
            Schedule::Cron(expr) => assert_eq!(expr, "0 */5 * * * *"),
            _ => panic!("expected Cron"),
        }
    }

    #[test]
    fn test_schedule_cron_invalid() {
        let result = Schedule::cron("not a cron");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("invalid cron"));
    }
}
