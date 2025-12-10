//! Core data types for the storage layer.
//!
//! This module defines the primary data structures used throughout the storage layer:
//!
//! - [`Metric`]: Time-series numeric data points with category/symbol indexing
//! - [`Event`]: Structured event records for alerts, errors, and audit logs
//! - [`EventType`]: Classification of event nature/handling
//! - [`Severity`]: Priority levels for event delivery

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum_macros::{AsRefStr, Display, EnumString};

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
    pub kind: EventKind,
    /// Delivery priority/urgency.
    pub severity: EventSeverity,
    /// Short human-readable description.
    pub message: String,
    /// Context snapshot as JSON.
    pub payload: Option<serde_json::Value>,
}

/// Event kind classification.
///
/// Categorizes events by their nature and intended handling.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumString, Display, AsRefStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum EventKind {
    /// Rule-triggered notification (e.g., price threshold crossed).
    Alert,
    /// Collector or system error requiring attention.
    Error,
    /// Internal system event (e.g., startup, shutdown).
    System,
    /// Audit trail entry for tracking actions.
    Audit,
}

/// Event severity classification.
///
/// Controls delivery priority and visual presentation of events.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumString, Display, AsRefStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum EventSeverity {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // =========================================================================
    // EventKind tests
    // =========================================================================

    #[test]
    fn test_event_kind_from_str_valid() {
        assert_eq!(EventKind::from_str("alert").unwrap(), EventKind::Alert);
        assert_eq!(EventKind::from_str("error").unwrap(), EventKind::Error);
        assert_eq!(EventKind::from_str("system").unwrap(), EventKind::System);
        assert_eq!(EventKind::from_str("audit").unwrap(), EventKind::Audit);
    }

    #[test]
    fn test_event_kind_from_str_case_insensitive() {
        assert_eq!(EventKind::from_str("ALERT").unwrap(), EventKind::Alert);
        assert_eq!(EventKind::from_str("Error").unwrap(), EventKind::Error);
        assert_eq!(EventKind::from_str("SYSTEM").unwrap(), EventKind::System);
        assert_eq!(EventKind::from_str("AuDiT").unwrap(), EventKind::Audit);
    }

    #[test]
    fn test_event_kind_from_str_invalid() {
        let result = EventKind::from_str("unknown");
        assert!(result.is_err());
        // Strum returns strum::ParseError
    }

    #[test]
    fn test_event_kind_as_str() {
        assert_eq!(EventKind::Alert.as_ref(), "alert");
        assert_eq!(EventKind::Error.as_ref(), "error");
        assert_eq!(EventKind::System.as_ref(), "system");
        assert_eq!(EventKind::Audit.as_ref(), "audit");
    }

    // =========================================================================
    // EventSeverity tests
    // =========================================================================

    #[test]
    fn test_event_severity_from_str_valid() {
        assert_eq!(
            EventSeverity::from_str("debug").unwrap(),
            EventSeverity::Debug
        );
        assert_eq!(
            EventSeverity::from_str("info").unwrap(),
            EventSeverity::Info
        );
        assert_eq!(
            EventSeverity::from_str("warn").unwrap(),
            EventSeverity::Warn
        );
        assert_eq!(
            EventSeverity::from_str("error").unwrap(),
            EventSeverity::Error
        );
        assert_eq!(
            EventSeverity::from_str("critical").unwrap(),
            EventSeverity::Critical
        );
    }

    #[test]
    fn test_event_severity_from_str_case_insensitive() {
        assert_eq!(
            EventSeverity::from_str("DEBUG").unwrap(),
            EventSeverity::Debug
        );
        assert_eq!(
            EventSeverity::from_str("Info").unwrap(),
            EventSeverity::Info
        );
        assert_eq!(
            EventSeverity::from_str("WARN").unwrap(),
            EventSeverity::Warn
        );
        assert_eq!(
            EventSeverity::from_str("CrItIcAl").unwrap(),
            EventSeverity::Critical
        );
    }

    #[test]
    fn test_event_severity_from_str_invalid() {
        let result = EventSeverity::from_str("fatal");
        assert!(result.is_err());
        // Strum returns strum::ParseError
    }

    #[test]
    fn test_event_severity_as_str() {
        assert_eq!(EventSeverity::Debug.as_ref(), "debug");
        assert_eq!(EventSeverity::Info.as_ref(), "info");
        assert_eq!(EventSeverity::Warn.as_ref(), "warn");
        assert_eq!(EventSeverity::Error.as_ref(), "error");
        assert_eq!(EventSeverity::Critical.as_ref(), "critical");
    }
}
