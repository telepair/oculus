//! Core data types for the storage layer.
//!
//! This module defines the primary data structures used throughout the storage layer:
//!
//! - [`Metric`]: Time-series numeric data points with category/symbol indexing
//! - [`Event`]: Structured event records for alerts, errors, and audit logs
//! - [`EventType`]: Classification of event nature/handling
//! - [`Severity`]: Priority levels for event delivery

use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A metric data point stored in the `metrics` table.
///
/// Metrics are time-series numeric values collected from various sources
/// (network probes, crypto markets, stock markets, etc.). Each metric has
/// a unique `symbol` identifier and optional `tags` for extended filtering.
///
/// # Example
///
/// ```
/// use oculus::{Metric};
/// use chrono::Utc;
/// use serde_json::json;
///
/// let metric = Metric {
///     ts: Utc::now(),
///     category: "crypto".to_string(),
///     symbol: "crypto.price.btc_usd".to_string(),
///     value: 100000.0,
///     tags: Some(json!({"exchange": "binance"})),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    /// Timestamp when the metric was generated (UTC).
    pub ts: DateTime<Utc>,
    /// Logical group for aggregation/filtering (e.g., "crypto", "network").
    pub category: String,
    /// Unique metric identifier (e.g., "crypto.price.btc_usd").
    pub symbol: String,
    /// Numeric value for aggregation.
    pub value: f64,
    /// Extended fields as JSON (e.g., {"region": "us"}).
    pub tags: Option<serde_json::Value>,
}

/// An event record stored in the `events` table.
///
/// Events represent discrete occurrences in the system, such as rule-triggered
/// alerts, collector errors, or audit logs. Each event has a source identifier,
/// type classification, and severity level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Auto-generated event identifier.
    pub id: Option<i64>,
    /// Event timestamp (UTC).
    pub ts: DateTime<Utc>,
    /// Event origin (e.g., "rule.btc_alert", "collector.network").
    pub source: String,
    /// Event nature/handling class.
    pub event_type: EventType,
    /// Delivery priority/urgency.
    pub severity: Severity,
    /// Short human-readable description.
    pub message: String,
    /// Context snapshot as JSON.
    pub payload: Option<serde_json::Value>,
}

/// Event type classification.
///
/// Categorizes events by their nature and intended handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    /// Rule-triggered notification (e.g., price threshold crossed).
    Alert,
    /// Collector or system error requiring attention.
    Error,
    /// Internal system event (e.g., startup, shutdown).
    System,
    /// Audit trail entry for tracking actions.
    Audit,
}

impl EventType {
    /// Convert to database string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Error => "error",
            Self::System => "system",
            Self::Audit => "audit",
        }
    }
}

impl FromStr for EventType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "alert" => Ok(Self::Alert),
            "error" => Ok(Self::Error),
            "system" => Ok(Self::System),
            "audit" => Ok(Self::Audit),
            _ => Ok(Self::System),
        }
    }
}

/// Event severity levels.
///
/// Controls delivery priority and visual presentation of events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Verbose diagnostic information (development only).
    Debug,
    /// Normal operational information.
    Info,
    /// Potential issue that may require attention.
    Warn,
    /// Error condition requiring investigation.
    Error,
    /// Severe failure requiring immediate action.
    Critical,
}

impl Severity {
    /// Convert to database string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }
}

impl FromStr for Severity {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            "critical" => Ok(Self::Critical),
            _ => Ok(Self::Info),
        }
    }
}
