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

use crate::storage::{EventQuery, EventReader, MetricQuery, MetricReader, SortOrder};

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
}

/// Query parameters for metrics API.
#[derive(Debug, Deserialize)]
pub struct MetricsQueryParams {
    pub category: Option<String>,
    pub symbol: Option<String>,
    pub limit: Option<u32>,
    pub order: Option<String>,
}

/// Query parameters for events API.
#[derive(Debug, Deserialize)]
pub struct EventsQueryParams {
    pub source: Option<String>,
    pub kind: Option<String>,
    pub severity: Option<String>,
    pub limit: Option<u32>,
    pub order: Option<String>,
}

/// Parse sort order from string.
fn parse_sort_order(s: Option<String>) -> Option<SortOrder> {
    s.and_then(|order| match order.to_lowercase().as_str() {
        "asc" => Some(SortOrder::Asc),
        "desc" => Some(SortOrder::Desc),
        _ => None,
    })
}

/// Create the Axum router with all routes.
pub fn create_router(state: AppState) -> Router {
    let app_state = Arc::new(state);

    Router::new()
        .route("/", get(dashboard_handler))
        .route("/api/health", get(health_handler))
        .route("/api/metrics", get(metrics_handler))
        .route("/api/events", get(events_handler))
        .nest_service("/static", ServeDir::new("templates/static"))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .layer(CorsLayer::permissive())
        .with_state(app_state)
}

use askama::Template;

/// Dashboard template.
#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate;

/// Dashboard homepage handler.
async fn dashboard_handler() -> impl IntoResponse {
    DashboardTemplate
}

/// Health check endpoint.
async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

/// Metrics API endpoint - returns HTML partial for HTMX.
async fn metrics_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MetricsQueryParams>,
) -> Response {
    let query = MetricQuery {
        category: params.category,
        symbol: params.symbol,
        start: None,
        end: None,
        limit: params.limit,
        order: parse_sort_order(params.order),
    };

    match state.metric_reader.query(query) {
        Ok(metrics) => {
            // Build HTML table
            let mut html = String::from(
                "<table><thead><tr><th>Time</th><th>Category</th><th>Symbol</th><th>Value</th></tr></thead><tbody>",
            );

            for metric in metrics {
                html.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.2}</td></tr>",
                    metric.ts.format("%Y-%m-%d %H:%M:%S"),
                    metric.category,
                    metric.symbol,
                    metric.value
                ));
            }

            html.push_str("</tbody></table>");
            Html(html).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to query metrics: {}", e),
        )
            .into_response(),
    }
}

/// Events API endpoint - returns HTML partial for HTMX.
async fn events_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<EventsQueryParams>,
) -> Response {
    // Parse kind and severity from strings
    let kind = params.kind.as_ref().and_then(|k| k.parse().ok());
    let severity = params.severity.as_ref().and_then(|s| s.parse().ok());

    let query = EventQuery {
        source: params.source,
        kind,
        severity,
        start: None,
        end: None,
        limit: params.limit,
        order: parse_sort_order(params.order),
    };

    match state.event_reader.query(query) {
        Ok(events) => {
            // Build HTML table with severity-based styling
            let mut html = String::from(
                "<table><thead><tr><th>Time</th><th>Source</th><th>Kind</th><th>Severity</th><th>Message</th></tr></thead><tbody>",
            );

            for event in events {
                let severity_class = match event.severity.as_ref() {
                    "critical" => "color: #dc2626;",
                    "error" => "color: #ea580c;",
                    "warn" => "color: #ca8a04;",
                    "info" => "color: #2563eb;",
                    _ => "",
                };

                html.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td style=\"{}\">{}</td><td>{}</td></tr>",
                    event.ts.format("%Y-%m-%d %H:%M:%S"),
                    event.source,
                    event.kind.as_ref(),
                    severity_class,
                    event.severity.as_ref(),
                    event.message
                ));
            }

            html.push_str("</tbody></table>");
            Html(html).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to query events: {}", e),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBuilder;
    use axum::http::Request;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn create_test_state() -> (AppState, crate::storage::StorageHandles) {
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

        // Return handles to keep tempdir alive
        (state, handles)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let (state, _handles) = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let (state, _handles) = create_test_state();
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
        let (state, _handles) = create_test_state();
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
