//! API Integration Tests for Oculus
//!
//! Comprehensive tests covering all HTTP API endpoints.

use axum::body::Body;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use oculus::StorageBuilder;
use oculus::server::{AppState, create_router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

// =============================================================================
// Test Helpers
// =============================================================================

/// Create test app state with in-memory database.
async fn create_test_state() -> (AppState, oculus::StorageHandles) {
    let handles = StorageBuilder::new("sqlite::memory:")
        .channel_capacity(100)
        .build()
        .await
        .expect("Failed to build storage");

    let state = AppState {
        metric_reader: handles.metric_reader.clone(),
        event_reader: handles.event_reader.clone(),
        collector_store: handles.collector_store.clone(),
    };

    (state, handles)
}

/// Start test server and return base URL.
async fn start_test_server() -> (String, oculus::StorageHandles) {
    let (state, handles) = create_test_state().await;
    let router = create_router(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind random port");
    let addr = listener.local_addr().expect("Failed to get local addr");

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (format!("http://{}", addr), handles)
}

/// Parse response body as JSON (unused but kept for reference).
#[allow(dead_code)]
async fn parse_json_body(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

// =============================================================================
// Health Probe Tests
// =============================================================================

#[tokio::test]
async fn test_health_probes() {
    let (base_url, handles) = start_test_server().await;
    let client = reqwest::Client::new();

    // Test /healthz (liveness)
    let resp = client
        .get(format!("{}/healthz", base_url))
        .send()
        .await
        .expect("Failed to send healthz request");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("Failed to parse healthz response");
    assert_eq!(body["status"], "ok");

    // Test /readyz (readiness)
    let resp = client
        .get(format!("{}/readyz", base_url))
        .send()
        .await
        .expect("Failed to send readyz request");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("Failed to parse readyz response");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["db"], "ready");

    handles.shutdown().await.unwrap();
}

// =============================================================================
// Metrics API Tests
// =============================================================================

#[tokio::test]
async fn test_metrics_api() {
    let (base_url, handles) = start_test_server().await;
    let client = reqwest::Client::new();

    // Test /api/metrics with various query params
    let resp = client
        .get(format!("{}/api/metrics?limit=10&order=desc", base_url))
        .send()
        .await
        .expect("Failed to fetch metrics");
    assert_eq!(resp.status(), 200);

    // Test with category filter
    let resp = client
        .get(format!(
            "{}/api/metrics?category=network.tcp&limit=5",
            base_url
        ))
        .send()
        .await
        .expect("Failed to fetch filtered metrics");
    assert_eq!(resp.status(), 200);

    // Test with time range filter
    let resp = client
        .get(format!("{}/api/metrics?range=1h", base_url))
        .send()
        .await
        .expect("Failed to fetch ranged metrics");
    assert_eq!(resp.status(), 200);

    handles.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_metrics_stats_api() {
    let (base_url, handles) = start_test_server().await;
    let client = reqwest::Client::new();

    // Test /api/metrics/stats
    let resp = client
        .get(format!("{}/api/metrics/stats", base_url))
        .send()
        .await
        .expect("Failed to fetch metrics stats");
    assert_eq!(resp.status(), 200);

    // Test with range parameter
    let resp = client
        .get(format!("{}/api/metrics/stats?range=24h", base_url))
        .send()
        .await
        .expect("Failed to fetch ranged metrics stats");
    assert_eq!(resp.status(), 200);

    handles.shutdown().await.unwrap();
}

// =============================================================================
// Events API Tests
// =============================================================================

#[tokio::test]
async fn test_events_api() {
    let (base_url, handles) = start_test_server().await;
    let client = reqwest::Client::new();

    // Test /api/events with various query params
    let resp = client
        .get(format!("{}/api/events?limit=10&order=desc", base_url))
        .send()
        .await
        .expect("Failed to fetch events");
    assert_eq!(resp.status(), 200);

    // Test with severity filter
    let resp = client
        .get(format!("{}/api/events?severity=error&limit=5", base_url))
        .send()
        .await
        .expect("Failed to fetch filtered events");
    assert_eq!(resp.status(), 200);

    // Test with time range filter
    let resp = client
        .get(format!("{}/api/events?range=7d", base_url))
        .send()
        .await
        .expect("Failed to fetch ranged events");
    assert_eq!(resp.status(), 200);

    handles.shutdown().await.unwrap();
}

// =============================================================================
// Collectors CRUD Tests
// =============================================================================

#[tokio::test]
async fn test_collectors_crud() {
    let (base_url, handles) = start_test_server().await;
    let client = reqwest::Client::new();

    // 1. Create collector via POST /api/collectors
    let create_body = json!({
        "type": "tcp",
        "name": "test-collector",
        "enabled": true,
        "group": "test",
        "config": {
            "host": "127.0.0.1",
            "port": 8080,
            "interval": "30s",
            "timeout": "5s"
        }
    });

    let resp = client
        .post(format!("{}/api/collectors", base_url))
        .json(&create_body)
        .send()
        .await
        .expect("Failed to create collector");
    assert_eq!(resp.status(), StatusCode::CREATED.as_u16());

    let created: Value = resp
        .json()
        .await
        .expect("Failed to parse created collector");
    assert_eq!(created["name"], "test-collector");
    assert_eq!(created["collector_type"], "tcp");

    // 2. List collectors via GET /api/collectors
    let resp = client
        .get(format!("{}/api/collectors", base_url))
        .send()
        .await
        .expect("Failed to list collectors");
    assert_eq!(resp.status(), 200);

    let collectors: Vec<Value> = resp.json().await.expect("Failed to parse collectors list");
    assert!(
        collectors.iter().any(|c| c["name"] == "test-collector"),
        "Created collector should be in list"
    );

    // 3. Get single collector via GET /api/collectors/{type}/{name}
    let resp = client
        .get(format!("{}/api/collectors/tcp/test-collector", base_url))
        .send()
        .await
        .expect("Failed to get collector");

    assert_eq!(resp.status(), 200);

    let collector: Value = resp.json().await.expect("Failed to parse collector");
    assert_eq!(collector["name"], "test-collector");

    // 4. Update collector via PUT /api/collectors/{type}/{name}
    let update_body = json!({
        "type": "tcp",
        "name": "test-collector",
        "enabled": false,
        "group": "updated",
        "config": {
            "host": "192.168.1.1",
            "port": 9090,
            "interval": "60s",
            "timeout": "10s"
        }
    });

    let resp = client
        .put(format!("{}/api/collectors/tcp/test-collector", base_url))
        .json(&update_body)
        .send()
        .await
        .expect("Failed to update collector");
    assert_eq!(resp.status(), 200);

    let updated: Value = resp
        .json()
        .await
        .expect("Failed to parse updated collector");
    assert!(!updated["enabled"].as_bool().unwrap());
    assert_eq!(updated["group"], "updated");

    // 5. Delete collector via DELETE /api/collectors/{type}/{name}
    let resp = client
        .delete(format!("{}/api/collectors/tcp/test-collector", base_url))
        .send()
        .await
        .expect("Failed to delete collector");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT.as_u16());

    // Verify deletion
    let resp = client
        .get(format!("{}/api/collectors/tcp/test-collector", base_url))
        .send()
        .await
        .expect("Failed to verify deletion");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND.as_u16());

    handles.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_collector_not_found() {
    let (base_url, handles) = start_test_server().await;
    let client = reqwest::Client::new();

    // Test GET non-existent collector
    let resp = client
        .get(format!(
            "{}/api/collectors/tcp/non-existent-collector",
            base_url
        ))
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND.as_u16());

    // Test DELETE non-existent collector
    let resp = client
        .delete(format!(
            "{}/api/collectors/tcp/non-existent-collector",
            base_url
        ))
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND.as_u16());

    handles.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_collector_invalid_type() {
    let (base_url, handles) = start_test_server().await;
    let client = reqwest::Client::new();

    // Test GET with invalid collector type
    let resp = client
        .get(format!(
            "{}/api/collectors/invalid-type/some-name",
            base_url
        ))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST.as_u16());

    // Test POST with invalid collector type
    let create_body = json!({
        "type": "invalid-type",
        "name": "test",
        "config": {}
    });

    let resp = client
        .post(format!("{}/api/collectors", base_url))
        .json(&create_body)
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST.as_u16());

    handles.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_collector_duplicate_create() {
    let (base_url, handles) = start_test_server().await;
    let client = reqwest::Client::new();

    let create_body = json!({
        "type": "ping",
        "name": "duplicate-test",
        "enabled": true,
        "group": "test",
        "config": {
            "host": "8.8.8.8",
            "interval": "30s",
            "timeout": "5s"
        }
    });

    // First create should succeed
    let resp = client
        .post(format!("{}/api/collectors", base_url))
        .json(&create_body)
        .send()
        .await
        .expect("Failed to create collector");
    assert_eq!(resp.status(), StatusCode::CREATED.as_u16());

    // Second create with same name should return 409 Conflict
    let resp = client
        .post(format!("{}/api/collectors", base_url))
        .json(&create_body)
        .send()
        .await
        .expect("Failed to send duplicate request");
    assert_eq!(resp.status(), StatusCode::CONFLICT.as_u16());

    handles.shutdown().await.unwrap();
}
