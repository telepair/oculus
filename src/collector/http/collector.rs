//! HTTP endpoint probe collector.
//!
//! Measures HTTP/HTTPS endpoint latency and validates response status.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::collector::{Collector, CollectorError, Schedule};
use crate::storage::{MetricCategory, MetricSeries, MetricValue, StaticTags, StorageWriter};

/// Default collection interval (30 seconds).
const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Default request timeout (10 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default expected HTTP status code.
const DEFAULT_EXPECTED_STATUS: u16 = 200;

/// Latency value indicating probe failure (connection error, timeout, etc.).
/// Using -1.0 to distinguish from valid 0ms latency.
const FAILURE_LATENCY_MS: f64 = -1.0;

fn default_enabled() -> bool {
    true
}

fn default_group() -> String {
    "default".to_string()
}

fn default_method() -> HttpMethod {
    HttpMethod::Get
}

fn default_expected_status() -> u16 {
    DEFAULT_EXPECTED_STATUS
}

fn default_timeout() -> Duration {
    DEFAULT_TIMEOUT
}

/// HTTP method for requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
    Head,
    Put,
    Delete,
    Options,
    Patch,
}

impl std::str::FromStr for HttpMethod {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "HEAD" => Ok(Self::Head),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            "OPTIONS" => Ok(Self::Options),
            "PATCH" => Ok(Self::Patch),
            _ => Err(()),
        }
    }
}

impl HttpMethod {
    /// Get the method name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Head => "HEAD",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Patch => "PATCH",
        }
    }
}

/// Configuration for HTTP endpoint probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// Unique name for this probe instance.
    pub name: String,
    /// Target URL (HTTP or HTTPS).
    pub url: String,
    /// Enable this collector (default: true).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Collector group for organization (default: "default").
    #[serde(default = "default_group")]
    pub group: String,
    /// HTTP method to use (default: GET).
    #[serde(default = "default_method")]
    pub method: HttpMethod,
    /// Expected HTTP status code (default: 200).
    #[serde(default = "default_expected_status")]
    pub expected_status: u16,
    /// Collection interval (mutually exclusive with cron).
    #[serde(default, with = "humantime_serde")]
    pub interval: Option<Duration>,
    /// Cron schedule expression (mutually exclusive with interval).
    #[serde(default)]
    pub cron: Option<String>,
    /// Request timeout (default: 10s).
    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub timeout: Duration,
    /// Static tags for identity.
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Request headers with environment variable substitution support.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Success conditions for advanced validation.
    #[serde(default)]
    pub success_conditions: Option<SuccessConditions>,
    /// Metrics to extract from response body.
    #[serde(default)]
    pub extract_metrics: Vec<MetricExtraction>,
}

/// Success conditions for HTTP probe validation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SuccessConditions {
    /// Expected status codes.
    #[serde(default)]
    pub status_codes: Option<Vec<u16>>,
    /// Required response headers.
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
    /// Body must contain this string.
    #[serde(default)]
    pub body_contains: Option<String>,
    /// JSONPath expression that must match.
    #[serde(default)]
    pub body_jsonpath: Option<String>,
}

/// Metric extraction configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricExtraction {
    /// Name for the extracted metric.
    pub name: String,
    /// JSONPath expression to extract value.
    pub jsonpath: String,
    /// Optional unit.
    #[serde(default)]
    pub unit: Option<String>,
}

impl HttpConfig {
    /// Create a new HTTP probe configuration.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            enabled: true,
            group: "default".to_string(),
            method: HttpMethod::default(),
            expected_status: DEFAULT_EXPECTED_STATUS,
            interval: Some(DEFAULT_INTERVAL),
            cron: None,
            timeout: DEFAULT_TIMEOUT,
            tags: BTreeMap::new(),
            description: None,
            headers: BTreeMap::new(),
            success_conditions: None,
            extract_metrics: Vec::new(),
        }
    }

    /// Get static tags as StaticTags type.
    pub fn static_tags(&self) -> &StaticTags {
        &self.tags
    }

    /// Get schedule from interval or cron.
    pub fn schedule(&self) -> Schedule {
        if let Some(ref cron_expr) = self.cron {
            Schedule::Cron(cron_expr.clone())
        } else {
            Schedule::Interval(self.interval.unwrap_or(DEFAULT_INTERVAL))
        }
    }

    /// Set the HTTP method.
    pub fn with_method(mut self, method: HttpMethod) -> Self {
        self.method = method;
        self
    }

    /// Set the expected HTTP status code.
    pub fn with_expected_status(mut self, status: u16) -> Self {
        self.expected_status = status;
        self
    }

    /// Set the collection interval.
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = Some(interval);
        self.cron = None;
        self
    }

    /// Set the cron schedule.
    pub fn with_cron(mut self, cron: impl Into<String>) -> Self {
        self.cron = Some(cron.into());
        self.interval = None;
        self
    }

    /// Set the schedule directly.
    pub fn with_schedule(mut self, schedule: Schedule) -> Self {
        match schedule {
            Schedule::Interval(d) => {
                self.interval = Some(d);
                self.cron = None;
            }
            Schedule::Cron(expr) => {
                self.cron = Some(expr);
                self.interval = None;
            }
        }
        self
    }

    /// Set the request timeout.
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

    /// Set request headers.
    pub fn with_headers(mut self, headers: BTreeMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Add a single request header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Set success conditions.
    pub fn with_success_conditions(mut self, conditions: SuccessConditions) -> Self {
        self.success_conditions = Some(conditions);
        self
    }

    /// Set metrics to extract.
    pub fn with_extract_metrics(mut self, metrics: Vec<MetricExtraction>) -> Self {
        self.extract_metrics = metrics;
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

/// HTTP endpoint probe collector.
///
/// Measures HTTP response latency and validates status codes.
pub struct HttpCollector {
    config: HttpConfig,
    writer: StorageWriter,
    series_id: u64,
    client: Client,
}

impl HttpCollector {
    /// Create a new HTTP collector with the given configuration and writer.
    ///
    /// # Errors
    /// Returns `CollectorError::Config` if the HTTP client cannot be built.
    pub fn new(config: HttpConfig, writer: StorageWriter) -> Result<Self, CollectorError> {
        let series_id = MetricSeries::compute_series_id(
            MetricCategory::NetworkHttp,
            &config.name,
            &config.url,
            config.static_tags(),
        );

        // Build HTTP client with timeout
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| CollectorError::Config(format!("Failed to build HTTP client: {}", e)))?;

        Ok(Self {
            config,
            writer,
            series_id,
            client,
        })
    }
}

impl std::fmt::Debug for HttpCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpCollector")
            .field("config", &self.config)
            .field("series_id", &self.series_id)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Collector for HttpCollector {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn category(&self) -> MetricCategory {
        MetricCategory::NetworkHttp
    }

    fn schedule(&self) -> Schedule {
        self.config.schedule()
    }

    fn upsert_metric_series(&self) -> Result<u64, CollectorError> {
        // Create series
        let series = MetricSeries::new(
            MetricCategory::NetworkHttp,
            self.config.name.clone(),
            self.config.url.clone(),
            self.config.static_tags().clone(),
            self.config.description.clone(),
        );

        self.writer.upsert_metric_series(series)?;
        Ok(self.series_id)
    }

    async fn collect(&self) -> Result<(), CollectorError> {
        let probe_timeout = self.config.timeout;

        // Build request based on method
        let mut request = match self.config.method {
            HttpMethod::Get => self.client.get(&self.config.url),
            HttpMethod::Post => self.client.post(&self.config.url),
            HttpMethod::Head => self.client.head(&self.config.url),
            HttpMethod::Put => self.client.put(&self.config.url),
            HttpMethod::Delete => self.client.delete(&self.config.url),
            HttpMethod::Options => self
                .client
                .request(reqwest::Method::OPTIONS, &self.config.url),
            HttpMethod::Patch => self.client.patch(&self.config.url),
        };

        // Add request headers
        for (key, value) in &self.config.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        // Measure request time
        let start = Instant::now();
        let result = timeout(probe_timeout, request.send()).await;
        let elapsed = start.elapsed();
        let duration_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;

        let (latency_ms, success, status_code, body_opt) = match result {
            Ok(Ok(response)) => {
                let status = response.status().as_u16();
                let response_headers = response.headers().clone();
                let ms = elapsed.as_secs_f64() * 1000.0;

                // Read body if needed for conditions or metric extraction
                let needs_body = self.config.success_conditions.as_ref().map_or(false, |sc| {
                    sc.body_contains.is_some() || sc.body_jsonpath.is_some()
                }) || !self.config.extract_metrics.is_empty();

                let body = if needs_body {
                    response.text().await.ok()
                } else {
                    None
                };

                // Evaluate success conditions
                let success = self.evaluate_success(status, &response_headers, body.as_deref());

                if success {
                    tracing::debug!(
                        name = %self.config.name,
                        url = %self.config.url,
                        latency_ms = ms,
                        status = status,
                        "HTTP probe successful"
                    );
                } else {
                    tracing::warn!(
                        name = %self.config.name,
                        url = %self.config.url,
                        latency_ms = ms,
                        status = status,
                        expected = self.config.expected_status,
                        "HTTP probe failed conditions"
                    );
                }

                (ms, success, Some(status), body)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    name = %self.config.name,
                    url = %self.config.url,
                    error = %e,
                    "HTTP probe failed"
                );
                (FAILURE_LATENCY_MS, false, None, None)
            }
            Err(_) => {
                tracing::warn!(
                    name = %self.config.name,
                    url = %self.config.url,
                    timeout_ms = probe_timeout.as_millis(),
                    "HTTP probe timed out"
                );
                (FAILURE_LATENCY_MS, false, None, None)
            }
        };

        // Create value with dynamic tags
        let mut value = MetricValue::new(self.series_id, latency_ms, success)
            .with_unit("ms")
            .with_duration_ms(duration_ms)
            .with_tag("method", self.config.method.as_str());

        if let Some(status) = status_code {
            value = value.with_tag("status_code", status.to_string());
        }

        self.writer.insert_metric_value(value)?;

        // Extract metrics from body if configured
        if let Some(body) = body_opt {
            self.extract_metrics_from_body(&body)?;
        }

        Ok(())
    }
}

impl HttpCollector {
    /// Evaluate success conditions against response.
    fn evaluate_success(
        &self,
        status: u16,
        response_headers: &reqwest::header::HeaderMap,
        body: Option<&str>,
    ) -> bool {
        // If no advanced conditions, use simple expected_status check
        let Some(ref conditions) = self.config.success_conditions else {
            return status == self.config.expected_status;
        };

        // Check status codes
        if let Some(ref allowed_statuses) = conditions.status_codes {
            if !allowed_statuses.contains(&status) {
                return false;
            }
        } else if status != self.config.expected_status {
            return false;
        }

        // Check required headers
        if let Some(ref required_headers) = conditions.headers {
            for (key, expected_value) in required_headers {
                match response_headers.get(key) {
                    Some(actual) => {
                        if actual.to_str().unwrap_or("") != expected_value {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
        }

        // Check body contains
        if let Some(ref substring) = conditions.body_contains {
            if let Some(body_text) = body {
                if !body_text.contains(substring) {
                    return false;
                }
            } else {
                return false;
            }
        }

        // Check body JSONPath
        if let Some(ref jsonpath_expr) = conditions.body_jsonpath {
            if let Some(body_text) = body {
                if !self.evaluate_jsonpath_condition(body_text, jsonpath_expr) {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }

    /// Evaluate JSONPath condition against body.
    fn evaluate_jsonpath_condition(&self, body: &str, jsonpath_expr: &str) -> bool {
        use serde_json_path::JsonPath;

        let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
            return false;
        };

        let Ok(path) = jsonpath_expr.parse::<JsonPath>() else {
            tracing::warn!(
                name = %self.config.name,
                jsonpath = %jsonpath_expr,
                "Invalid JSONPath expression"
            );
            return false;
        };

        let nodes = path.query(&json);

        // Consider truthy if any nodes matched and are not null/false/empty
        if nodes.is_empty() {
            return false;
        }

        nodes.iter().any(|node| match node {
            serde_json::Value::Null => false,
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(s) => !s.is_empty(),
            serde_json::Value::Array(a) => !a.is_empty(),
            serde_json::Value::Object(o) => !o.is_empty(),
            serde_json::Value::Number(_) => true,
        })
    }

    /// Extract metrics from response body using JSONPath.
    fn extract_metrics_from_body(&self, body: &str) -> Result<(), CollectorError> {
        use serde_json_path::JsonPath;

        if self.config.extract_metrics.is_empty() {
            return Ok(());
        }

        let json: serde_json::Value = serde_json::from_str(body).map_err(|e| {
            CollectorError::Config(format!("Failed to parse response body as JSON: {}", e))
        })?;

        for metric in &self.config.extract_metrics {
            let path = metric.jsonpath.parse::<JsonPath>().map_err(|e| {
                CollectorError::Config(format!("Invalid JSONPath '{}': {}", metric.jsonpath, e))
            })?;

            let nodes = path.query(&json);
            if let Some(node) = nodes.first() {
                let node_ref: &serde_json::Value = node;
                if let Some(value) = node_ref.as_f64() {
                    tracing::debug!(
                        name = %self.config.name,
                        metric_name = %metric.name,
                        value = value,
                        "Extracted metric from response"
                    );
                    // TODO: Store extracted metric - requires additional series registration
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_config_defaults() {
        let config = HttpConfig::new("api-health", "https://api.example.com/health");

        assert_eq!(config.name, "api-health");
        assert_eq!(config.url, "https://api.example.com/health");
        assert_eq!(config.method, HttpMethod::Get);
        assert_eq!(config.expected_status, DEFAULT_EXPECTED_STATUS);
        assert!(matches!(config.schedule(), Schedule::Interval(d) if d == DEFAULT_INTERVAL));
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_http_config_builder() {
        let config = HttpConfig::new("api-post", "https://api.example.com/submit")
            .with_method(HttpMethod::Post)
            .with_expected_status(201)
            .with_interval(Duration::from_secs(60))
            .with_timeout(Duration::from_secs(5))
            .with_description("API submission endpoint");

        assert_eq!(config.method, HttpMethod::Post);
        assert_eq!(config.expected_status, 201);
        assert!(matches!(config.schedule(), Schedule::Interval(d) if d == Duration::from_secs(60)));
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(
            config.description,
            Some("API submission endpoint".to_string())
        );
    }

    #[test]
    fn test_http_method_from_str() {
        assert_eq!("GET".parse::<HttpMethod>().ok(), Some(HttpMethod::Get));
        assert_eq!("get".parse::<HttpMethod>().ok(), Some(HttpMethod::Get));
        assert_eq!("POST".parse::<HttpMethod>().ok(), Some(HttpMethod::Post));
        assert_eq!("head".parse::<HttpMethod>().ok(), Some(HttpMethod::Head));
        assert_eq!("put".parse::<HttpMethod>().ok(), Some(HttpMethod::Put));
        assert_eq!(
            "DELETE".parse::<HttpMethod>().ok(),
            Some(HttpMethod::Delete)
        );
        assert_eq!(
            "options".parse::<HttpMethod>().ok(),
            Some(HttpMethod::Options)
        );
        assert_eq!("PATCH".parse::<HttpMethod>().ok(), Some(HttpMethod::Patch));
        assert_eq!("INVALID".parse::<HttpMethod>().ok(), None);
    }

    #[test]
    fn test_http_method_as_str() {
        assert_eq!(HttpMethod::Get.as_str(), "GET");
        assert_eq!(HttpMethod::Post.as_str(), "POST");
        assert_eq!(HttpMethod::Head.as_str(), "HEAD");
    }
}
