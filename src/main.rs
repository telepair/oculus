//! Oculus Binary Entry Point
//!
//! This binary runs the complete Oculus monitoring system.
//! Core functionality is provided by the `oculus` library crate.

use clap::Parser;
use oculus::{
    collector::network::{TcpCollector, TcpConfig},
    collector::{CollectorRegistry, Schedule},
    config::{AppConfig, parse_duration},
    server::{AppState, create_router},
    storage::{Event, EventKind, EventSeverity, EventSource, StorageBuilder},
};
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Oculus - Unified Telemetry System
#[derive(Parser, Debug)]
#[command(name = "oculus", version, about, long_about = None)]
struct Cli {
    /// Path to configuration file
    #[arg(
        short,
        long,
        default_value = "configs/config.yaml",
        env = "OCULUS_CONFIG"
    )]
    config: String,

    /// Server bind address (overrides config file)
    #[arg(long, env = "OCULUS_SERVER_BIND")]
    server_bind: Option<String>,

    /// Server port (overrides config file)
    #[arg(long, env = "OCULUS_SERVER_PORT")]
    server_port: Option<u16>,

    /// Database path (overrides config file)
    #[arg(long, env = "OCULUS_DB_PATH")]
    db_path: Option<String>,

    /// Database pool size (overrides config file)
    #[arg(long, env = "OCULUS_DB_POOL_SIZE")]
    db_pool_size: Option<u32>,
}

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

    // Parse CLI arguments
    let cli = Cli::parse();

    // Load configuration from file
    tracing::info!("Loading configuration from: {}", cli.config);
    let mut config = AppConfig::load(&cli.config)?;

    // Apply CLI/env overrides (CLI > ENV > config file)
    if let Some(bind) = cli.server_bind {
        config.server.bind = bind;
    }
    if let Some(port) = cli.server_port {
        config.server.port = port;
    }
    if let Some(path) = cli.db_path {
        config.database.path = path;
    }
    if let Some(pool_size) = cli.db_pool_size {
        config.database.pool_size = pool_size;
    }

    tracing::info!(
        "Server: {}:{}, Database: {}, Collectors: {}",
        config.server.bind,
        config.server.port,
        config.database.path,
        config.collectors.len()
    );

    // Build storage layer
    tracing::info!("Initializing storage...");
    let checkpoint_interval = parse_duration(&config.database.checkpoint_interval)?;
    let handles = StorageBuilder::new(&config.database.path)
        .pool_size(config.database.pool_size)
        .channel_capacity(config.database.channel_capacity)
        .checkpoint_interval(checkpoint_interval)
        .build()?;

    tracing::info!("Storage initialized at: {}", config.database.path);

    // Initialize collector registry
    tracing::info!("Starting collector registry...");
    let registry = CollectorRegistry::new(handles.writer.clone()).await?;
    registry.start().await?;

    // Emit service started event
    if let Err(e) = handles.writer.insert_event(Event::new(
        EventSource::System,
        EventKind::System,
        EventSeverity::Info,
        "Service started",
    )) {
        tracing::warn!("Failed to emit service start event: {}", e);
    }

    // Spawn collectors from configuration
    for collector_config in &config.collectors {
        match collector_config.collector_type {
            oculus::MetricCategory::NetworkTcp => {
                // Parse timeout
                let timeout = parse_duration(&collector_config.timeout)?;

                // Parse schedule (interval or cron)
                let schedule = if let Some(ref interval) = collector_config.interval {
                    Schedule::interval(parse_duration(interval)?)
                } else if let Some(ref cron_expr) = collector_config.cron {
                    Schedule::cron(cron_expr).map_err(|e| format!("Invalid cron: {}", e))?
                } else {
                    return Err("Collector must have either 'interval' or 'cron'".into());
                };

                // Parse target address
                let target = collector_config.target.parse().map_err(|e| {
                    format!(
                        "Invalid target address '{}': {}",
                        collector_config.target, e
                    )
                })?;

                // Create TCP collector config with tags
                let tcp_config = TcpConfig::new(&collector_config.name, target)
                    .with_schedule(schedule.clone())
                    .with_timeout(timeout)
                    .with_static_tags(collector_config.tags.clone());

                // Create and spawn collector
                let collector = TcpCollector::new(tcp_config, handles.writer.clone());
                registry.spawn(collector).await.map_err(|e| {
                    format!(
                        "Failed to spawn collector '{}': {}",
                        collector_config.name, e
                    )
                })?;

                tracing::info!(
                    "Spawned collector: {} (tcp, target={}, schedule={})",
                    collector_config.name,
                    collector_config.target,
                    schedule
                );
            }
            other => {
                tracing::warn!(
                    "Unsupported collector type '{:?}' for collector '{}', skipping",
                    other,
                    collector_config.name
                );
            }
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

    // Emit service stopping event
    if let Err(e) = handles.writer.insert_event(Event::new(
        EventSource::System,
        EventKind::System,
        EventSeverity::Info,
        "Service stopping",
    )) {
        tracing::warn!("Failed to emit service stop event: {}", e);
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
