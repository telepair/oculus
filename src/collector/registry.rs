//! Collector registry for managing collector lifecycle.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::collector::{Collector, CollectorConfig, CollectorError, Schedule};
use crate::{Event, EventKind, EventSeverity, StorageWriter};

/// Default timeout for graceful shutdown (5 seconds).
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// Static event source tag for registry events.
const REGISTRY_EVENT_SOURCE: &str = "collector.registry";

/// Metadata about a registered job.
#[derive(Debug, Clone)]
pub struct JobInfo {
    /// Job UUID.
    pub id: uuid::Uuid,
    /// Collector name.
    pub name: String,
    /// Schedule description.
    pub schedule: String,
}

/// Registry for managing multiple collector tasks.
///
/// Uses `tokio-cron-scheduler` for robust job scheduling.
/// Supports both fixed-interval and cron-based scheduling.
pub struct CollectorRegistry {
    scheduler: JobScheduler,
    jobs: Arc<RwLock<HashMap<uuid::Uuid, JobInfo>>>,
    writer: StorageWriter,
}

impl CollectorRegistry {
    /// Create a new collector registry.
    pub async fn new(writer: StorageWriter) -> Result<Self, CollectorError> {
        let scheduler = JobScheduler::new()
            .await
            .map_err(|e| CollectorError::Scheduler(e.to_string()))?;

        Ok(Self {
            scheduler,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            writer,
        })
    }
}

impl std::fmt::Debug for CollectorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollectorRegistry")
            .field(
                "job_count",
                &self.jobs.try_read().map(|j| j.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

impl CollectorRegistry {
    /// Register and spawn a collector.
    pub async fn spawn<C: Collector>(&self, collector: C) -> Result<uuid::Uuid, CollectorError> {
        let name = collector.config().name().to_string();
        let schedule_desc = collector.config().schedule().to_string();
        let category = collector.category().to_string();

        let collector = Arc::new(collector);
        let job = self
            .create_job(Arc::clone(&collector), &name)
            .map_err(|e| CollectorError::Scheduler(e.to_string()))
            .inspect_err(|e| self.emit_job_error(&name, &category, &schedule_desc, e, "create"))?;

        let job_id = self
            .scheduler
            .add(job)
            .await
            .map_err(|e| CollectorError::Scheduler(e.to_string()))
            .inspect_err(|e| {
                self.emit_job_error(&name, &category, &schedule_desc, e, "register")
            })?;

        self.jobs.write().await.insert(
            job_id,
            JobInfo {
                id: job_id,
                name: name.clone(),
                schedule: schedule_desc.clone(),
            },
        );

        self.emit_info(
            format!("Job '{}' created", name),
            serde_json::json!({
                "job_id": job_id.to_string(),
                "collector": name,
                "category": category,
                "schedule": schedule_desc,
            }),
        );

        tracing::info!(collector = %name, job_id = %job_id, "Collector registered");
        Ok(job_id)
    }

    /// Start the scheduler.
    pub async fn start(&self) -> Result<(), CollectorError> {
        self.scheduler
            .start()
            .await
            .map_err(|e| CollectorError::Scheduler(e.to_string()))?;
        self.emit_info("Collector scheduler started", serde_json::json!({}));
        tracing::info!("Collector scheduler started");
        Ok(())
    }

    /// List all registered jobs.
    pub async fn list_jobs(&self) -> Vec<JobInfo> {
        self.jobs.read().await.values().cloned().collect()
    }

    /// Get the number of registered jobs.
    pub async fn job_count(&self) -> usize {
        self.jobs.read().await.len()
    }

    /// Gracefully shutdown the scheduler with default timeout.
    pub async fn shutdown(self) -> Result<(), CollectorError> {
        self.shutdown_with_timeout(DEFAULT_SHUTDOWN_TIMEOUT).await
    }

    /// Shutdown with custom timeout.
    pub async fn shutdown_with_timeout(mut self, timeout: Duration) -> Result<(), CollectorError> {
        let job_count = self.jobs.read().await.len();
        let shutdown_result = tokio::time::timeout(timeout, async {
            self.scheduler
                .shutdown()
                .await
                .map_err(|e| CollectorError::Scheduler(e.to_string()))
        })
        .await;

        let mut timed_out = false;
        if let Some(err) = match shutdown_result {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => {
                timed_out = true;
                None
            }
        } {
            self.emit(
                EventSeverity::Error,
                "Collector scheduler shutdown failed",
                serde_json::json!({ "job_count": job_count, "error": err.to_string() }),
            );
            return Err(err);
        }

        let (severity, msg) = if timed_out {
            tracing::warn!("Collector scheduler shutdown timed out");
            (
                EventSeverity::Warn,
                "Collector scheduler shutdown timed out",
            )
        } else {
            tracing::info!("Collector scheduler shutdown complete");
            (EventSeverity::Info, "Collector scheduler shutdown complete")
        };

        self.emit(
            severity,
            msg,
            serde_json::json!({ "job_count": job_count, "timed_out": timed_out }),
        );
        Ok(())
    }

    /// Remove a specific collector job by ID.
    pub async fn remove(&self, job_id: &uuid::Uuid) -> Result<(), CollectorError> {
        let job_name = self.jobs.read().await.get(job_id).map(|j| j.name.clone());

        self.scheduler
            .remove(job_id)
            .await
            .map_err(|e| CollectorError::Scheduler(e.to_string()))
            .inspect_err(|e| {
                self.emit(
                    EventSeverity::Error,
                    format!("Job '{}' remove failed", job_id),
                    serde_json::json!({
                        "job_id": job_id.to_string(),
                        "collector": job_name,
                        "error": e.to_string(),
                    }),
                );
            })?;

        self.jobs.write().await.remove(job_id);

        self.emit_info(
            format!("Job '{}' removed", job_id),
            serde_json::json!({ "job_id": job_id.to_string(), "collector": job_name }),
        );
        tracing::info!(job_id = %job_id, "Collector removed");
        Ok(())
    }

    // --- Private helpers ---

    fn create_job<C: Collector>(
        &self,
        collector: Arc<C>,
        name: &str,
    ) -> Result<Job, CollectorError> {
        let name = name.to_owned();
        let writer = self.writer.clone();
        let schedule = collector.config().schedule().clone();

        let make_callback = move || {
            let (collector, name, writer) = (Arc::clone(&collector), name.clone(), writer.clone());
            move |_: uuid::Uuid, _: JobScheduler| {
                let (collector, name, writer) =
                    (Arc::clone(&collector), name.clone(), writer.clone());
                Box::pin(async move { run_collection(&collector, &name, &writer).await })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            }
        };

        match &schedule {
            Schedule::Interval(d) => Job::new_repeated_async(*d, make_callback()),
            Schedule::Cron(expr) => Job::new_cron_job_async(expr, make_callback()),
        }
        .map_err(|e| CollectorError::Scheduler(e.to_string()))
    }

    fn emit(
        &self,
        severity: EventSeverity,
        message: impl Into<String>,
        payload: serde_json::Value,
    ) {
        let event = Event {
            id: None,
            ts: Utc::now(),
            source: REGISTRY_EVENT_SOURCE.to_owned(),
            kind: EventKind::System,
            severity,
            message: message.into(),
            payload: Some(payload),
        };
        if let Err(e) = self.writer.insert_event(event) {
            tracing::warn!(error = %e, "Failed to enqueue event");
        }
    }

    fn emit_info(&self, message: impl Into<String>, payload: serde_json::Value) {
        self.emit(EventSeverity::Info, message, payload);
    }

    fn emit_job_error(
        &self,
        name: &str,
        category: &str,
        schedule: &str,
        err: &CollectorError,
        stage: &str,
    ) {
        self.emit(
            EventSeverity::Error,
            format!("Job '{}' {} failed", name, stage),
            serde_json::json!({
                "collector": name,
                "category": category,
                "schedule": schedule,
                "error": err.to_string(),
                "stage": stage,
            }),
        );
    }
}

/// Execute a single collection cycle and record the result.
async fn run_collection<C: Collector>(collector: &Arc<C>, name: &str, writer: &StorageWriter) {
    let start = std::time::Instant::now();
    tracing::debug!(collector = %name, "Running collection");

    let result = collector.collect().await;
    let duration_ms = start.elapsed().as_millis();

    let (kind, severity, message, status) = match &result {
        Ok(()) => {
            tracing::debug!(collector = %name, duration_ms, "Collection succeeded");
            (
                EventKind::System,
                EventSeverity::Debug,
                format!("Collection completed in {}ms", duration_ms),
                "success",
            )
        }
        Err(e) => {
            tracing::error!(collector = %name, error = %e, "Collection failed");
            (
                EventKind::Error,
                EventSeverity::Error,
                format!("Collection failed: {}", e),
                "failed",
            )
        }
    };

    let mut payload =
        serde_json::json!({ "collector": name, "duration_ms": duration_ms, "status": status });
    if let Err(e) = &result {
        payload["error"] = serde_json::json!(e.to_string());
    }

    let event = Event {
        id: None,
        ts: Utc::now(),
        source: format!("collector.{}", name),
        kind,
        severity,
        message,
        payload: Some(payload),
    };
    if let Err(e) = writer.insert_event(event) {
        tracing::warn!(error = %e, "Failed to enqueue collection event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{StorageBuilder, StorageWriter};

    /// A mock collector for testing.
    struct MockCollector {
        config: MockConfig,
        #[allow(dead_code)]
        writer: StorageWriter,
    }

    #[derive(Clone)]
    struct MockConfig {
        name: String,
        schedule: Schedule,
    }

    impl MockConfig {
        fn new(name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                schedule: Schedule::interval(Duration::from_secs(60)),
            }
        }
    }

    impl CollectorConfig for MockConfig {
        fn name(&self) -> &str {
            &self.name
        }

        fn schedule(&self) -> &Schedule {
            &self.schedule
        }

        fn timeout(&self) -> Duration {
            Duration::from_secs(5)
        }
    }

    #[async_trait::async_trait]
    impl Collector for MockCollector {
        type Config = MockConfig;

        fn new(config: Self::Config, writer: StorageWriter) -> Self {
            Self { config, writer }
        }

        fn category(&self) -> &str {
            "test"
        }

        fn config(&self) -> &Self::Config {
            &self.config
        }

        async fn collect(&self) -> Result<(), CollectorError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_registry_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let handles = StorageBuilder::new(&db_path).build().unwrap();
        let writer = handles.writer.clone();
        let registry = CollectorRegistry::new(writer.clone()).await.unwrap();

        let config = MockConfig::new("test-collector");
        let collector = MockCollector::new(config, writer);

        // Spawn collector
        let job_id = registry.spawn(collector).await.unwrap();
        assert_eq!(registry.job_count().await, 1);

        // List jobs
        let jobs = registry.list_jobs().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "test-collector");
        assert!(jobs[0].schedule.contains("60s"));

        // Remove collector
        registry.remove(&job_id).await.unwrap();
        assert_eq!(registry.job_count().await, 0);

        // Shutdown
        registry.shutdown().await.unwrap();
        handles.shutdown().unwrap();
    }

    #[tokio::test]
    async fn test_schedule_cron_validation() {
        // Invalid cron now fails at Schedule construction time
        let result = Schedule::cron("invalid cron expression");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid cron"));
    }
}
