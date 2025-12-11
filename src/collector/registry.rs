//! Collector registry for managing collector lifecycle.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::collector::{Collector, CollectorError, Schedule};
use crate::storage::MetricCategory;
use crate::{Event, EventKind, EventPayload, EventSeverity, EventSource, StorageWriter};

/// Default timeout for graceful shutdown (5 seconds).
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Metadata about a registered job.
#[derive(Debug, Clone)]
pub struct JobInfo {
    /// Job UUID.
    pub id: uuid::Uuid,
    /// Category of the collector.
    pub category: MetricCategory,
    /// Collector name.
    pub name: String,
    /// Metric series ID.
    pub metric_series_id: u64,
    /// Schedule description.
    pub schedule: String,
}

struct JobContext<C> {
    collector: Arc<C>,
    name: String,
    category: MetricCategory,
    writer: StorageWriter,
}

/// Registry for managing multiple collector tasks.
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
    ///
    /// This method:
    /// 1. Calls `upsert_series()` to register the metric series
    /// 2. Creates a scheduled job that calls `collect()` repeatedly
    pub async fn spawn<C: Collector>(&self, collector: C) -> Result<uuid::Uuid, CollectorError> {
        let name = collector.name().to_string();
        let schedule_desc = collector.schedule().to_string();
        let category = collector.category();
        let category_str = category.as_ref();

        // Upsert metric series during registration
        let series_id = collector.upsert_metric_series().inspect_err(|e| {
            self.emit_job_error(
                &name,
                category_str,
                0,
                &schedule_desc.clone(),
                e,
                "upsert_series",
            )
        })?;

        let collector = Arc::new(collector);
        let job = self
            .create_job(Arc::clone(&collector), &name)
            .map_err(|e| CollectorError::Scheduler(e.to_string()))
            .inspect_err(|e| {
                self.emit_job_error(
                    &name,
                    category_str,
                    series_id,
                    &schedule_desc.clone(),
                    e,
                    "create",
                )
            })?;

        let job_id = self
            .scheduler
            .add(job)
            .await
            .map_err(|e| CollectorError::Scheduler(e.to_string()))
            .inspect_err(|e| {
                self.emit_job_error(
                    &name,
                    category_str,
                    series_id,
                    &schedule_desc.clone(),
                    e,
                    "register",
                )
            })?;

        self.jobs.write().await.insert(
            job_id,
            JobInfo {
                id: job_id,
                category,
                name: name.clone(),
                metric_series_id: series_id,
                schedule: schedule_desc.clone(),
            },
        );

        tracing::info!(
            category = category_str,
            collector = %name,
            series_id = series_id,
            job_id = %job_id,
            schedule = %schedule_desc,
            "Metric series registered"
        );

        self.emit_info(format!("Job created: {}", name), |p| {
            p.insert("job_id".into(), job_id.to_string());
            p.insert("collector".into(), name.clone());
            p.insert("category".into(), category_str.to_string());
            p.insert("schedule".into(), schedule_desc);
            p.insert("series_id".into(), series_id.to_string());
        });

        tracing::info!(collector = %name, job_id = %job_id, "Collector registered");
        Ok(job_id)
    }

    /// Start the scheduler.
    pub async fn start(&self) -> Result<(), CollectorError> {
        self.scheduler
            .start()
            .await
            .map_err(|e| CollectorError::Scheduler(e.to_string()))?;
        self.emit_info("Collector scheduler started", |_| {});
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

    /// Gracefully shutdown the scheduler.
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

        match shutdown_result {
            Ok(Ok(())) => {
                tracing::info!("Collector scheduler shutdown complete");
                self.emit(EventSeverity::Info, "Scheduler shutdown complete", |p| {
                    p.insert("job_count".into(), job_count.to_string());
                    p.insert("timed_out".into(), "false".into());
                });
                Ok(())
            }
            Ok(Err(err)) => {
                self.emit(EventSeverity::Error, "Scheduler shutdown failed", |p| {
                    p.insert("job_count".into(), job_count.to_string());
                    p.insert("error".into(), err.to_string());
                });
                Err(err)
            }
            Err(_) => {
                let err = CollectorError::Scheduler("scheduler shutdown timed out".to_string());
                tracing::error!("Collector scheduler shutdown timed out");
                self.emit(EventSeverity::Error, "Scheduler shutdown timed out", |p| {
                    p.insert("job_count".into(), job_count.to_string());
                    p.insert("timed_out".into(), "true".into());
                });
                Err(err)
            }
        }
    }

    /// Remove a specific collector job by ID.
    pub async fn remove(&self, job_id: &uuid::Uuid) -> Result<(), CollectorError> {
        let job_name = self.jobs.read().await.get(job_id).map(|j| j.name.clone());

        self.scheduler
            .remove(job_id)
            .await
            .map_err(|e| CollectorError::Scheduler(e.to_string()))
            .inspect_err(|e| {
                self.emit(EventSeverity::Error, "Job remove failed", |p| {
                    p.insert("job_id".into(), job_id.to_string());
                    if let Some(ref name) = job_name {
                        p.insert("collector".into(), name.clone());
                    }
                    p.insert("error".into(), e.to_string());
                });
            })?;

        self.jobs.write().await.remove(job_id);

        self.emit_info("Job removed", |p| {
            p.insert("job_id".into(), job_id.to_string());
            if let Some(ref name) = job_name {
                p.insert("collector".into(), name.clone());
            }
        });
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
        let category = collector.category();
        let writer = self.writer.clone();
        let schedule = collector.schedule().clone();

        let context = Arc::new(JobContext {
            collector,
            name,
            category,
            writer,
        });

        let make_callback = move || {
            let ctx = Arc::clone(&context);
            move |_: uuid::Uuid, _: JobScheduler| {
                let ctx = Arc::clone(&ctx);
                Box::pin(async move { run_collection(ctx).await })
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
        build_payload: impl FnOnce(&mut EventPayload),
    ) {
        let mut payload = EventPayload::new();
        build_payload(&mut payload);
        let event = Event::new(
            EventSource::CollectorRegistry,
            EventKind::System,
            severity,
            message,
        )
        .with_payloads(payload);
        if let Err(e) = self.writer.insert_event(event) {
            tracing::warn!(error = %e, "Failed to enqueue event");
        }
    }

    fn emit_info(&self, message: impl Into<String>, build_payload: impl FnOnce(&mut EventPayload)) {
        self.emit(EventSeverity::Info, message, build_payload);
    }

    fn emit_job_error(
        &self,
        name: &str,
        category: &str,
        metric_series_id: u64,
        schedule: &str,
        err: &CollectorError,
        stage: &str,
    ) {
        self.emit(
            EventSeverity::Error,
            format!("Job '{}' {} failed", name, stage),
            |p| {
                p.insert("collector".into(), name.to_string());
                p.insert("category".into(), category.to_string());
                p.insert("metric_series_id".into(), metric_series_id.to_string());
                p.insert("schedule".into(), schedule.to_string());
                p.insert("error".into(), err.to_string());
                p.insert("stage".into(), stage.to_string());
            },
        );
    }
}

/// Map category to EventSource.
fn category_to_event_source(category: MetricCategory) -> EventSource {
    match category {
        MetricCategory::NetworkTcp => EventSource::CollectorNetworkTcp,
        MetricCategory::NetworkPing => EventSource::CollectorNetworkPing,
        MetricCategory::NetworkHttp => EventSource::CollectorNetworkHttp,
        _ => EventSource::Other,
    }
}

/// Execute a single collection cycle and record the result.
async fn run_collection<C: Collector>(ctx: Arc<JobContext<C>>) {
    let start = std::time::Instant::now();
    tracing::debug!(collector = %ctx.name, "Running collection");

    let result = ctx.collector.collect().await;
    let duration_ms = start.elapsed().as_millis();

    if let Ok(()) = result {
        tracing::debug!(
            collector = %ctx.name,
            duration_ms,
            "Collection succeeded"
        );
        // Do not record successful events to the database to avoid noise
        return;
    }

    // Handle failure case
    let e = result.unwrap_err(); // Safe because we handled Ok above
    tracing::error!(collector = %ctx.name, error = %e, "Collection failed");

    let event_source = category_to_event_source(ctx.category);
    let event = Event::new(
        event_source,
        EventKind::Error,
        EventSeverity::Error,
        format!("Collection failed: {}", e),
    )
    .with_payload("collector", &ctx.name)
    .with_payload("category", ctx.category.as_ref())
    .with_payload("duration_ms", duration_ms.to_string())
    .with_payload("status", "failed")
    .with_payload("error", e.to_string());

    if let Err(e) = ctx.writer.insert_event(event) {
        tracing::warn!(error = %e, "Failed to enqueue collection event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{MetricSeries, MetricValue, StaticTags, StorageBuilder, StorageWriter};

    /// A mock collector for testing.
    struct MockCollector {
        name: String,
        schedule: Schedule,
        writer: StorageWriter,
        series_id: u64,
    }

    impl MockCollector {
        fn new(name: impl Into<String>, writer: StorageWriter) -> Self {
            let name = name.into();
            // Compute a deterministic series_id for testing
            let series_id = crate::storage::MetricSeries::compute_series_id(
                MetricCategory::Custom,
                "mock",
                &name,
                &StaticTags::new(),
            );
            Self {
                name,
                schedule: Schedule::interval(Duration::from_secs(60)),
                writer,
                series_id,
            }
        }
    }

    #[async_trait::async_trait]
    impl Collector for MockCollector {
        fn name(&self) -> &str {
            &self.name
        }

        fn category(&self) -> MetricCategory {
            MetricCategory::Custom
        }

        fn schedule(&self) -> &Schedule {
            &self.schedule
        }

        fn upsert_metric_series(&self) -> Result<u64, CollectorError> {
            let series = MetricSeries::new(
                MetricCategory::Custom,
                "mock",
                &self.name,
                StaticTags::new(),
                Some("Mock collector for testing".to_string()),
            );
            self.writer.upsert_metric_series(series)?;
            Ok(self.series_id)
        }

        async fn collect(&self) -> Result<(), CollectorError> {
            // Insert a simple metric value
            let value = MetricValue::new(self.series_id, 1.0, true, 0);
            self.writer.insert_metric_value(value)?;
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

        let collector = MockCollector::new("test-collector", writer);

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
