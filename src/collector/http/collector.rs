//! HTTP endpoint probe collector.
//!
//! Measures HTTP/HTTPS endpoint latency and validates response status.

use std::time::{Duration, Instant};

use reqwest::Client;
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

/// HTTP method for requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Unique name for this probe instance.
    pub name: String,
    /// Target URL (HTTP or HTTPS).
    pub url: String,
    /// HTTP method to use.
    pub method: HttpMethod,
    /// Expected HTTP status code (default: 200).
    pub expected_status: u16,
    /// Static tags for identity.
    pub static_tags: StaticTags,
    /// Human-readable description.
    pub description: Option<String>,
    /// Collection interval.
    pub interval: Duration,
    /// Request timeout.
    pub timeout: Duration,
}

impl HttpConfig {
    /// Create a new HTTP probe configuration.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            method: HttpMethod::default(),
            expected_status: DEFAULT_EXPECTED_STATUS,
            interval: DEFAULT_INTERVAL,
            timeout: DEFAULT_TIMEOUT,
            static_tags: StaticTags::new(),
            description: None,
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
        self.interval = interval;
        self
    }

    /// Set the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
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
    pub fn new(config: HttpConfig, writer: StorageWriter) -> Self {
        let series_id = MetricSeries::compute_series_id(
            MetricCategory::NetworkHttp,
            &config.name,
            &config.url,
            &config.static_tags,
        );

        // Build HTTP client with timeout
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            config,
            writer,
            series_id,
            client,
        }
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
        Schedule::Interval(self.config.interval)
    }

    fn upsert_metric_series(&self) -> Result<u64, CollectorError> {
        // Create series
        let series = MetricSeries::new(
            MetricCategory::NetworkHttp,
            self.config.name.clone(),
            self.config.url.clone(),
            self.config.static_tags.clone(),
            self.config.description.clone(),
        );

        self.writer.upsert_metric_series(series)?;
        Ok(self.series_id)
    }

    async fn collect(&self) -> Result<(), CollectorError> {
        let probe_timeout = self.config.timeout;

        // Build request based on method
        let request = match self.config.method {
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

        // Measure request time
        let start = Instant::now();
        let result = timeout(probe_timeout, request.send()).await;
        let elapsed = start.elapsed();
        let duration_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;

        let (latency_ms, success, status_code) = match result {
            Ok(Ok(response)) => {
                let status = response.status().as_u16();
                let status_matches = status == self.config.expected_status;
                let ms = elapsed.as_secs_f64() * 1000.0;

                if status_matches {
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
                        "HTTP probe returned unexpected status"
                    );
                }

                (ms, status_matches, Some(status))
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    name = %self.config.name,
                    url = %self.config.url,
                    error = %e,
                    "HTTP probe failed"
                );
                (FAILURE_LATENCY_MS, false, None)
            }
            Err(_) => {
                tracing::warn!(
                    name = %self.config.name,
                    url = %self.config.url,
                    timeout_ms = probe_timeout.as_millis(),
                    "HTTP probe timed out"
                );
                (FAILURE_LATENCY_MS, false, None)
            }
        };

        // Create value with dynamic tags
        let mut value = MetricValue::new(self.series_id, latency_ms, success)
            .with_unit("ms")
            .with_duration_ms(duration_ms)
            .with_tag("method", self.config.method.as_str())
            .with_tag("interval", format!("{}s", self.config.interval.as_secs()));

        if let Some(status) = status_code {
            value = value.with_tag("status_code", status.to_string());
        }

        self.writer.insert_metric_value(value)?;
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
        assert_eq!(config.interval, DEFAULT_INTERVAL);
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
        assert_eq!(config.interval, Duration::from_secs(60));
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
