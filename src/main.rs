//! Oculus Binary Entry Point
//!
//! This binary runs the complete Oculus monitoring system.
//! Core functionality is provided by the `oculus` library crate.

use oculus::{
    AppConfig, AppState, Collector, CollectorRegistry, Schedule, StorageBuilder, TcpCollector,
    TcpConfig, create_router,
};
use std::env;
use std::net::SocketAddr;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,oculus=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Oculus - Unified Telemetry System");

    // Load configuration
    let config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "configs/config.yaml".to_string());

    tracing::info!("Loading configuration from: {}", config_path);
    let config = AppConfig::load(&config_path)?;

    tracing::info!(
        "Server: {}:{}, Database: {}, Collectors: {}",
        config.server.bind,
        config.server.port,
        config.database.path,
        config.collectors.len()
    );

    // Build storage layer
    tracing::info!("Initializing storage...");
    let handles = StorageBuilder::new(&config.database.path)
        .pool_size(config.database.pool_size)
        .channel_capacity(config.database.channel_capacity)
        .build()?;

    tracing::info!("Storage initialized at: {}", config.database.path);

    // Initialize collector registry
    tracing::info!("Starting collector registry...");
    let registry = CollectorRegistry::new(handles.writer.clone()).await?;
    registry.start().await?;

    // Spawn collectors from configuration
    for collector_config in &config.collectors {
        if collector_config.collector_type == "tcp" {
            // Parse interval and timeout
            let interval = parse_duration(&collector_config.interval)?;
            let timeout = parse_duration(&collector_config.timeout)?;

            // Parse target address
            let target = collector_config.target.parse().map_err(|e| {
                format!(
                    "Invalid target address '{}': {}",
                    collector_config.target, e
                )
            })?;

            // Create TCP collector config
            let tcp_config = TcpConfig::new(&collector_config.name, target)
                .with_schedule(Schedule::interval(interval))
                .with_timeout(timeout);

            // Create and spawn collector
            let collector = TcpCollector::new(tcp_config, handles.writer.clone());
            registry.spawn(collector).await.map_err(|e| {
                format!(
                    "Failed to spawn collector '{}': {}",
                    collector_config.name, e
                )
            })?;

            tracing::info!(
                "Spawned collector: {} (tcp, target={}, interval={:?})",
                collector_config.name,
                collector_config.target,
                interval
            );
        } else {
            tracing::warn!(
                "Unknown collector type '{}' for collector '{}', skipping",
                collector_config.collector_type,
                collector_config.name
            );
        }
    }

    // Create web server state
    let app_state = AppState {
        metric_reader: handles.metric_reader.clone(),
        event_reader: handles.event_reader.clone(),
    };

    // Build Axum router
    let app = create_router(app_state);

    // Parse bind address
    let addr: SocketAddr = format!("{}:{}", config.server.bind, config.server.port).parse()?;

    tracing::info!("Web server listening on: http://{}", addr);
    tracing::info!("Press Ctrl+C to shutdown");

    // Start server with graceful shutdown
    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(registry, handles))
        .await?;

    tracing::info!("Shutdown complete");
    Ok(())
}

/// Parse duration string (e.g., "30s", "1m", "5m").
fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("duration string is empty".to_string());
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number: {}", num_str))?;

    match unit {
        "s" => Ok(Duration::from_secs(num)),
        "m" => Ok(Duration::from_secs(num * 60)),
        "h" => Ok(Duration::from_secs(num * 3600)),
        _ => Err(format!("invalid unit: {}. Use 's', 'm', or 'h'", unit)),
    }
}

/// Setup graceful shutdown signal handler.
async fn shutdown_signal(registry: CollectorRegistry, handles: oculus::StorageHandles) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C signal");
        }
        _ = terminate => {
            tracing::info!("Received terminate signal");
        }
    }

    tracing::info!("Shutting down collectors...");
    if let Err(e) = registry.shutdown().await {
        tracing::error!("Failed to shutdown collectors: {}", e);
    }

    tracing::info!("Shutting down storage...");
    if let Err(e) = handles.shutdown() {
        tracing::error!("Failed to shutdown storage: {}", e);
    }
}
