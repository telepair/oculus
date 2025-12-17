//! Oculus Binary Entry Point
//!
//! This binary runs the complete Oculus monitoring system.
//! Core functionality is provided by the `oculus` library crate.

use clap::Parser;
use oculus::{
    collector::CollectorRegistry,
    collector::http::HttpCollector,
    collector::ping::PingCollector,
    collector::tcp::TcpCollector,
    config::{AppConfig, CollectorsConfig},
    server::{AppState, create_router},
    storage::{
        CollectorRecord, CollectorType, Event, EventKind, EventSeverity, EventSource,
        StorageBuilder,
    },
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

    /// Database URL (overrides config file)
    #[arg(long, env = "OCULUS_DB_URL")]
    db_url: Option<String>,
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
    if let Some(dsn) = cli.db_url {
        config.database.dsn = dsn;
    }

    tracing::info!(
        "Server: {}:{}, Database: {} ({})",
        config.server.bind,
        config.server.port,
        config.database.dsn,
        config.database.driver,
    );

    // Build storage layer
    let db_url = config.database.connection_url();
    tracing::info!("Initializing storage at: {}", db_url);

    let handles = StorageBuilder::new(&db_url)
        .channel_capacity(config.database.channel_capacity)
        .build()
        .await?;

    tracing::info!("Storage initialized");

    // Sync collectors from include directory to database (insert only, no updates)
    if let Some(ref include_path) = config.collector_include {
        tracing::info!("Loading collectors from: {}", include_path);
        let collectors_config = CollectorsConfig::load_from_dir(include_path)?;
        collectors_config.validate()?;

        let records = collectors_config.to_collector_records();
        let mut inserted = 0;
        let mut skipped = 0;

        for record in &records {
            match handles.collector_store.insert_if_not_exists(record).await? {
                Some(id) => {
                    tracing::info!(
                        "Synced collector: {} ({}, id={})",
                        record.name,
                        record.collector_type.as_ref(),
                        id
                    );
                    inserted += 1;
                }
                None => {
                    tracing::debug!(
                        "Collector already exists, skipping: {} ({})",
                        record.name,
                        record.collector_type.as_ref()
                    );
                    skipped += 1;
                }
            }
        }

        tracing::info!(
            "Collector sync complete: {} inserted, {} skipped",
            inserted,
            skipped
        );
    }

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

    // Load and spawn collectors from database
    let db_collectors = handles.collector_store.list_all().await?;
    tracing::info!("Found {} collectors in database", db_collectors.len());

    for record in db_collectors {
        if !record.enabled {
            tracing::debug!(
                "Skipping disabled collector: {} ({})",
                record.name,
                record.collector_type.as_ref()
            );
            continue;
        }

        match spawn_collector(&record, &handles.writer, &registry).await {
            Ok(()) => {
                tracing::info!(
                    "Spawned collector: {} ({})",
                    record.name,
                    record.collector_type.as_ref()
                );
            }
            Err(e) => {
                tracing::error!("Failed to spawn collector '{}': {}", record.name, e);
            }
        }
    }

    // Create web server state
    let app_state = AppState {
        metric_reader: handles.metric_reader.clone(),
        event_reader: handles.event_reader.clone(),
        collector_store: handles.collector_store.clone(),
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

/// Spawn a collector from a database record.
async fn spawn_collector(
    record: &CollectorRecord,
    writer: &oculus::StorageWriter,
    registry: &CollectorRegistry,
) -> Result<(), Box<dyn std::error::Error>> {
    match record.collector_type {
        CollectorType::Tcp => {
            let config: oculus::TcpConfig = serde_json::from_value(record.config.clone())?;
            let collector = TcpCollector::new(config, writer.clone());
            registry.spawn(collector).await?;
        }
        CollectorType::Ping => {
            let config: oculus::PingConfig = serde_json::from_value(record.config.clone())?;
            let collector = PingCollector::new(config, writer.clone());
            registry.spawn(collector).await?;
        }
        CollectorType::Http => {
            let config: oculus::collector::http::HttpConfig =
                serde_json::from_value(record.config.clone())?;
            let collector = HttpCollector::new(config, writer.clone())?;
            registry.spawn(collector).await?;
        }
    }
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
    if let Err(e) = handles.shutdown().await {
        tracing::error!("Failed to shutdown storage: {}", e);
    }
}
