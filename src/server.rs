//! Web server module for Oculus.
//!
//! Provides HTTP API endpoints and serves the HTMX-powered web UI.

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::{
    cors::CorsLayer,
    services::ServeDir,
    trace::{DefaultMakeSpan, TraceLayer},
};

use crate::storage::{
    Event, EventQuery, EventReader, EventSource, MetricCategory, MetricQuery, MetricReader,
    SortOrder,
};

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub metric_reader: MetricReader,
    pub event_reader: EventReader,
}

/// Health check response.
#[derive(Serialize)]
struct HealthResponse {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    db: Option<String>,
}

/// Query parameters for metrics API.
#[derive(Debug, Deserialize)]
pub struct MetricsQueryParams {
    pub category: Option<String>,
    pub name: Option<String>,
    pub target: Option<String>,
    pub limit: Option<u32>,
    pub order: Option<String>,
    pub range: Option<String>,
}

/// Query parameters for events API.
#[derive(Debug, Deserialize)]
pub struct EventsQueryParams {
    pub source: Option<String>,
    pub kind: Option<String>,
    pub severity: Option<String>,
    pub limit: Option<u32>,
    pub order: Option<String>,
    pub range: Option<String>,
}

/// Query parameters for metrics stats API.
#[derive(Debug, Deserialize)]
pub struct StatsQueryParams {
    pub range: Option<String>,
}

/// Parse sort order from string.
fn parse_sort_order(s: Option<String>) -> Option<SortOrder> {
    s.and_then(|order| match order.to_lowercase().as_str() {
        "asc" => Some(SortOrder::Asc),
        "desc" => Some(SortOrder::Desc),
        _ => None,
    })
}

/// Parse filtered time range from string.
/// Supports: 1h, 6h, 12h, 24h, 7d, 30d.
fn parse_range(range: Option<String>) -> Option<chrono::DateTime<chrono::Utc>> {
    let range = range?;
    let now = chrono::Utc::now();
    match range.as_str() {
        "1h" => Some(now - chrono::Duration::hours(1)),
        "6h" => Some(now - chrono::Duration::hours(6)),
        "12h" => Some(now - chrono::Duration::hours(12)),
        "24h" => Some(now - chrono::Duration::hours(24)),
        "7d" => Some(now - chrono::Duration::days(7)),
        "30d" => Some(now - chrono::Duration::days(30)),
        _ => None,
    }
}

use askama::Template;

/// Dashboard template.
#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate;

/// Metrics table partial template.
#[derive(Template)]
#[template(path = "partials/metrics.html")]
struct MetricsTemplate {
    metrics: Vec<crate::storage::MetricResult>,
}

/// Events table partial template.
#[derive(Template)]
#[template(path = "partials/events.html")]
struct EventsTemplate {
    events: Vec<Event>,
}

impl EventsTemplate {}

/// Wrapper to render Askama templates as Axum responses.
struct HtmlTemplate<T>(T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(rendered) => Html(rendered).into_response(),
            Err(err) => {
                tracing::error!(error = %err, "Template render failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

/// Create the Axum router with all routes.
pub fn create_router(state: AppState) -> Router {
    let app_state = Arc::new(state);

    Router::new()
        .route("/", get(dashboard_handler))
        .route("/healthz", get(healthz_handler))
        .route("/readyz", get(readyz_handler))
        .route("/api/metrics", get(metrics_handler))
        .route("/api/metrics/stats", get(metrics_stats_handler))
        .route("/api/events", get(events_handler))
        .nest_service("/static", ServeDir::new("templates/static"))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .layer(CorsLayer::permissive())
        .with_state(app_state)
}

/// Dashboard homepage handler.
async fn dashboard_handler() -> impl IntoResponse {
    DashboardTemplate
}

/// Liveness probe.
async fn healthz_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        db: None,
    })
}

/// Readiness probe that checks DuckDB availability.
async fn readyz_handler(State(state): State<Arc<AppState>>) -> Response {
    let db_status = state
        .metric_reader
        .query(MetricQuery {
            limit: Some(1),
            ..Default::default()
        })
        .map(|_| "ready".to_string())
        .map_err(|e| e.to_string());

    match db_status {
        Ok(db) => Json(HealthResponse {
            status: "ok".to_string(),
            db: Some(db),
        })
        .into_response(),
        Err(err) => {
            tracing::error!(error = %err, "Readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    status: "not_ready".to_string(),
                    db: Some(err),
                }),
            )
                .into_response()
        }
    }
}

/// Metrics API endpoint - returns HTML partial for HTMX.
async fn metrics_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MetricsQueryParams>,
) -> Response {
    let category = params
        .category
        .as_ref()
        .and_then(|c| c.parse::<MetricCategory>().ok());

    let query = MetricQuery {
        category,
        name: params.name.filter(|s| !s.is_empty()),
        target: params.target.filter(|s| !s.is_empty()),
        start: parse_range(params.range),
        end: None,
        limit: params.limit,
        order: parse_sort_order(params.order),
    };

    match state.metric_reader.query(query) {
        Ok(metrics) => HtmlTemplate(MetricsTemplate { metrics }).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)).into_response(),
    }
}

/// Metrics stats API endpoint - returns JSON statistics.
async fn metrics_stats_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StatsQueryParams>,
) -> Response {
    let start = parse_range(params.range);
    match state.metric_reader.stats(start, None) {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)).into_response(),
    }
}

/// Events API endpoint - returns HTML partial for HTMX.
async fn events_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<EventsQueryParams>,
) -> Response {
    let source = params
        .source
        .as_ref()
        .and_then(|s| s.parse::<EventSource>().ok());
    let kind = params.kind.as_ref().and_then(|k| k.parse().ok());
    let severity = params.severity.as_ref().and_then(|s| s.parse().ok());

    let query = EventQuery {
        source,
        kind,
        severity,
        start: parse_range(params.range),
        end: None,
        limit: params.limit,
        order: parse_sort_order(params.order),
    };

    match state.event_reader.query(query) {
        Ok(events) => HtmlTemplate(EventsTemplate { events }).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBuilder;
    use axum::http::Request;
    use tempfile::{TempDir, tempdir};
    use tower::ServiceExt;

    fn create_test_state() -> (AppState, crate::storage::StorageHandles, TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_server.db");

        let handles = StorageBuilder::new(&db_path)
            .pool_size(2)
            .channel_capacity(100)
            .build()
            .expect("Failed to build storage");

        // Checkpoint to ensure schema is visible to readers
        handles.admin.checkpoint().expect("Failed to checkpoint");

        let state = AppState {
            metric_reader: handles.metric_reader.clone(),
            event_reader: handles.event_reader.clone(),
        };

        // Return handles AND dir to keep tempdir alive
        (state, handles, dir)
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let (state, _handles, _dir) = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics?limit=10")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        if status != StatusCode::OK {
            use axum::body::to_bytes;
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body = String::from_utf8_lossy(&bytes);
            panic!("Expected 200 OK, got {}. Body: {}", status, body);
        }
    }

    #[tokio::test]
    async fn test_events_endpoint() {
        let (state, _handles, _dir) = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/events?limit=10")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
