//! Core data types for the storage layer.
//!
//! This module defines the primary data structures for Series-Values separation:
//!
//! - [`MetricCategory`]: Classification of metric sources
//! - [`MetricSeries`]: Dimension table for static metric metadata
//! - [`MetricValue`]: Time-series data points with dynamic tags
//! - [`Event`]: Structured event records for alerts, errors, and audit logs

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum_macros::{AsRefStr, Display, EnumString};
use xxhash_rust::xxh64::xxh64;

// =============================================================================
// Metric Types
// =============================================================================

/// Metric category classification.
///
/// Used to partition metrics by data source type.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, Display, AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case", ascii_case_insensitive)]
pub enum MetricCategory {
    /// TCP port probe metrics.
    #[strum(serialize = "network.tcp")]
    #[serde(rename = "network.tcp")]
    NetworkTcp,
    /// ICMP ping probe metrics.
    #[strum(serialize = "network.ping")]
    #[serde(rename = "network.ping")]
    NetworkPing,
    /// HTTP endpoint probe metrics.
    #[strum(serialize = "network.http")]
    #[serde(rename = "network.http")]
    NetworkHttp,
    /// Cryptocurrency market data.
    Crypto,
    /// Polymarket prediction data.
    Polymarket,
    /// Stock market data.
    Stock,
    /// User-defined custom metrics.
    Custom,
}

/// Static tags type (sorted for consistent hashing).
pub type StaticTags = BTreeMap<String, String>;

/// Dynamic tags type.
pub type DynamicTags = BTreeMap<String, String>;

/// Metric series (dimension table).
///
/// Stores static metadata that identifies a unique metric series.
/// The `series_id` is computed as xxhash64 of (category, name, target, sorted_static_tags).
///
/// # Deduplication
///
/// Series are deduplicated using `INSERT OR REPLACE` on `series_id`.
/// When a duplicate series is inserted, only `updated_at` and `description` are refreshed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSeries {
    /// Unique identifier: xxhash64(category|name|target|sorted_static_tags).
    pub series_id: u64,
    /// Metric category.
    pub category: MetricCategory,
    /// Metric name (e.g., "latency", "price").
    pub name: String,
    /// Target identifier (e.g., "127.0.0.1:6379", "BTC/USD").
    pub target: String,
    /// Static tags for identity (e.g., {"host": "prod-1", "region": "us-west"}).
    pub static_tags: StaticTags,
    /// Human-readable description.
    pub description: Option<String>,
    /// First seen timestamp.
    pub created_at: DateTime<Utc>,
    /// Last updated timestamp.
    pub updated_at: DateTime<Utc>,
}

impl MetricSeries {
    /// Compute series_id from category, name, target, and static_tags.
    ///
    /// Formula: xxhash64("category|name|target|k1=v1,k2=v2,...")
    /// Tags are sorted by key for consistent hashing.
    pub fn compute_series_id(
        category: MetricCategory,
        name: &str,
        target: &str,
        static_tags: &StaticTags,
    ) -> u64 {
        let tags_str = static_tags
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        let input = format!("{}|{}|{}|{}", category.as_ref(), name, target, tags_str);
        xxh64(input.as_bytes(), 0)
    }

    /// Create a new MetricSeries with computed series_id.
    pub fn new(
        category: MetricCategory,
        name: impl Into<String>,
        target: impl Into<String>,
        static_tags: StaticTags,
        description: Option<String>,
    ) -> Self {
        let name = name.into();
        let target = target.into();
        let series_id = Self::compute_series_id(category, &name, &target, &static_tags);
        let now = Utc::now();
        Self {
            series_id,
            category,
            name,
            target,
            static_tags,
            description,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Metric value (data table).
///
/// Stores time-series numeric data linked to a series via `series_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricValue {
    /// Timestamp when the value was recorded.
    pub ts: DateTime<Utc>,
    /// Reference to the metric series.
    pub series_id: u64,
    /// Numeric value.
    pub value: f64,
    /// Unit of the value (e.g., "ms", "USD", "%", "bytes").
    pub unit: Option<String>,
    /// Whether the collection was successful.
    pub success: bool,
    /// Collection duration in milliseconds.
    pub duration_ms: u32,
    /// Dynamic tags for context (e.g., {"status_code": "200", "trace_id": "abc123"}).
    pub dynamic_tags: DynamicTags,
}

impl MetricValue {
    /// Create a new metric value.
    pub fn new(series_id: u64, value: f64, success: bool) -> Self {
        Self {
            ts: Utc::now(),
            series_id,
            value,
            unit: None,
            success,
            duration_ms: 0,
            dynamic_tags: DynamicTags::new(),
        }
    }

    /// Set the unit of the value.
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Set the duration of the collection in milliseconds.
    pub fn with_duration_ms(mut self, duration_ms: u32) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    /// Add a dynamic tag.
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.dynamic_tags.insert(key.into(), value.into());
        self
    }

    /// Add multiple dynamic tags.
    pub fn with_tags(mut self, tags: DynamicTags) -> Self {
        self.dynamic_tags.extend(tags);
        self
    }
}

// =============================================================================
// Event Types
// =============================================================================

/// Event payload type (flat key-value pairs).
pub type EventPayload = BTreeMap<String, String>;

/// Event source classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, Display, AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case", ascii_case_insensitive)]
pub enum EventSource {
    /// TCP collector events.
    #[strum(serialize = "collector.network.tcp")]
    #[serde(rename = "collector.network.tcp")]
    CollectorNetworkTcp,
    /// Ping collector events.
    #[strum(serialize = "collector.network.ping")]
    #[serde(rename = "collector.network.ping")]
    CollectorNetworkPing,
    /// HTTP collector events.
    #[strum(serialize = "collector.network.http")]
    #[serde(rename = "collector.network.http")]
    CollectorNetworkHttp,
    /// Rule engine events.
    #[strum(serialize = "rule.engine")]
    #[serde(rename = "rule.engine")]
    RuleEngine,
    /// System events.
    System,
}

/// Event kind classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, Display, AsRefStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum EventKind {
    /// Rule-triggered notification.
    Alert,
    /// Collector or system error.
    Error,
    /// Internal system event.
    System,
    /// Audit trail entry.
    Audit,
}

/// Event severity classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, Display, AsRefStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum EventSeverity {
    /// Verbose diagnostic information.
    Debug,
    /// Normal operational information.
    Info,
    /// Potential issue requiring attention.
    Warn,
    /// Error condition requiring investigation.
    Error,
    /// Severe failure requiring immediate action.
    Critical,
}

/// An event record stored in the `events` table (wide table design).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Auto-generated event identifier.
    pub id: Option<i64>,
    /// Event timestamp (UTC).
    pub ts: DateTime<Utc>,
    /// Event source (enum compressed).
    pub source: EventSource,
    /// Event kind (enum compressed).
    pub kind: EventKind,
    /// Event severity (enum compressed).
    pub severity: EventSeverity,
    /// Short human-readable message.
    pub message: String,
    /// Flat key-value payload (stored as JSON VARCHAR).
    pub payload: EventPayload,
}

impl Event {
    /// Create a new event.
    pub fn new(
        source: EventSource,
        kind: EventKind,
        severity: EventSeverity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id: None,
            ts: Utc::now(),
            source,
            kind,
            severity,
            message: message.into(),
            payload: EventPayload::new(),
        }
    }

    /// Add a payload key-value pair.
    pub fn with_payload(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.payload.insert(key.into(), value.into());
        self
    }

    /// Add multiple payload entries.
    pub fn with_payloads(mut self, entries: EventPayload) -> Self {
        self.payload.extend(entries);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // =========================================================================
    // MetricCategory tests
    // =========================================================================

    #[test]
    fn test_metric_category_from_str() {
        assert_eq!(
            MetricCategory::from_str("network.tcp").unwrap(),
            MetricCategory::NetworkTcp
        );
        assert_eq!(
            MetricCategory::from_str("crypto").unwrap(),
            MetricCategory::Crypto
        );
        assert_eq!(
            MetricCategory::from_str("CUSTOM").unwrap(),
            MetricCategory::Custom
        );
    }

    #[test]
    fn test_metric_category_as_ref() {
        assert_eq!(MetricCategory::NetworkTcp.as_ref(), "network.tcp");
        assert_eq!(MetricCategory::Crypto.as_ref(), "crypto");
    }

    // =========================================================================
    // MetricSeries tests
    // =========================================================================

    #[test]
    fn test_series_id_computation() {
        let mut tags = StaticTags::new();
        tags.insert("host".to_string(), "prod-1".to_string());
        tags.insert("region".to_string(), "us-west".to_string());

        let id1 = MetricSeries::compute_series_id(
            MetricCategory::NetworkTcp,
            "latency",
            "127.0.0.1:6379",
            &tags,
        );

        // Same inputs should produce same ID
        let id2 = MetricSeries::compute_series_id(
            MetricCategory::NetworkTcp,
            "latency",
            "127.0.0.1:6379",
            &tags,
        );
        assert_eq!(id1, id2);

        // Different target should produce different ID
        let id3 = MetricSeries::compute_series_id(
            MetricCategory::NetworkTcp,
            "latency",
            "127.0.0.1:6380",
            &tags,
        );
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_series_id_tag_order_independence() {
        // BTreeMap ensures consistent ordering
        let mut tags1 = StaticTags::new();
        tags1.insert("a".to_string(), "1".to_string());
        tags1.insert("b".to_string(), "2".to_string());

        let mut tags2 = StaticTags::new();
        tags2.insert("b".to_string(), "2".to_string());
        tags2.insert("a".to_string(), "1".to_string());

        let id1 = MetricSeries::compute_series_id(MetricCategory::Custom, "test", "target", &tags1);
        let id2 = MetricSeries::compute_series_id(MetricCategory::Custom, "test", "target", &tags2);

        assert_eq!(id1, id2, "Tag insertion order should not affect series_id");
    }

    #[test]
    fn test_metric_series_new() {
        let tags = StaticTags::new();
        let series = MetricSeries::new(
            MetricCategory::NetworkTcp,
            "latency",
            "127.0.0.1:6379",
            tags,
            Some("Redis latency".to_string()),
        );

        assert!(series.series_id > 0);
        assert_eq!(series.category, MetricCategory::NetworkTcp);
        assert_eq!(series.name, "latency");
        assert_eq!(series.target, "127.0.0.1:6379");
    }

    // =========================================================================
    // MetricValue tests
    // =========================================================================

    #[test]
    fn test_metric_value_builder() {
        let value = MetricValue::new(12345, 100.5, true)
            .with_unit("ms")
            .with_duration_ms(15)
            .with_tag("status_code", "200")
            .with_tag("path", "/api/v1");

        assert_eq!(value.series_id, 12345);
        assert_eq!(value.value, 100.5);
        assert_eq!(value.unit, Some("ms".to_string()));
        assert!(value.success);
        assert_eq!(value.duration_ms, 15);
        assert_eq!(
            value.dynamic_tags.get("status_code"),
            Some(&"200".to_string())
        );
    }

    // =========================================================================
    // EventKind tests
    // =========================================================================

    #[test]
    fn test_event_kind_from_str() {
        assert_eq!(EventKind::from_str("alert").unwrap(), EventKind::Alert);
        assert_eq!(EventKind::from_str("ERROR").unwrap(), EventKind::Error);
    }

    #[test]
    fn test_event_kind_as_str() {
        assert_eq!(EventKind::Alert.as_ref(), "alert");
        assert_eq!(EventKind::System.as_ref(), "system");
    }

    // =========================================================================
    // EventSeverity tests
    // =========================================================================

    #[test]
    fn test_event_severity_from_str() {
        assert_eq!(
            EventSeverity::from_str("critical").unwrap(),
            EventSeverity::Critical
        );
        assert_eq!(
            EventSeverity::from_str("WARN").unwrap(),
            EventSeverity::Warn
        );
    }

    #[test]
    fn test_event_severity_as_str() {
        assert_eq!(EventSeverity::Debug.as_ref(), "debug");
        assert_eq!(EventSeverity::Critical.as_ref(), "critical");
    }
}
