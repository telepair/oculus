# Library Integration Guide

This guide covers using Oculus as a Rust library for custom telemetry applications.

## Quick Start

Add to `Cargo.toml`:

```toml
[dependencies]
oculus = { git = "https://github.com/telepair/oculus.git" }
chrono = "0.4"
```

## Basic Usage

```rust
use oculus::{StorageBuilder, Metric, MetricReader};
use chrono::Utc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build storage (spawns writer actor thread)
    let handles = StorageBuilder::new("./data.db")
        .pool_size(4)           // Read connection pool size
        .channel_capacity(1024) // Writer command queue size
        .build()?;

    // Insert metrics via unified writer facade
    handles.writer.insert_metric(Metric {
        ts: Utc::now(),
        category: "custom".to_string(),
        symbol: "my.metric".to_string(),
        value: 42.0,
        tags: None,
    })?;

    // Force WAL checkpoint for immediate read visibility
    handles.admin.checkpoint()?;

    // Query via reader facade
    let results = handles.metric_reader.query(Default::default())?;
    println!("Found {} metrics", results.len());

    // Graceful shutdown
    handles.shutdown()?;
    Ok(())
}
```

## API Overview

### Writer (MPSC Channel → Single Writer Thread)

| Facade | Methods | Description |
|--------|---------|-------------|
| `StorageWriter` | `insert_metric()`, `insert_metrics()` | Write metrics |
| | `insert_event()`, `insert_events()` | Write events |

### Readers (r2d2 Connection Pool)

| Facade | Methods | Description |
|--------|---------|-------------|
| `MetricReader` | `query(MetricQuery)` | Query metrics with filters |
| `EventReader` | `query(EventQuery)` | Query events with filters |
| `RawSqlReader` | `execute(sql)` | Execute raw SQL queries |

### Admin

| Facade | Methods | Description |
|--------|---------|-------------|
| `StorageAdmin` | `cleanup_metrics()`, `cleanup_events()`, `checkpoint()`, `shutdown()` | Maintenance operations |

## Query Examples

```rust
use oculus::facades::{MetricQuery, SortOrder};
use chrono::{Utc, Duration};

// Query recent crypto metrics
let results = handles.metric_reader.query(MetricQuery {
    category: Some("crypto".to_string()),
    start: Some(Utc::now() - Duration::hours(1)),
    limit: Some(50),
    order: Some(SortOrder::Desc),
    ..Default::default()
})?;

// Raw SQL query
let rows = handles.raw_sql_reader.execute(
    "SELECT symbol, AVG(value) as avg FROM metrics GROUP BY symbol"
)?;
```

## Error Handling

All operations return `Result<T, StorageError>`:

```rust
use oculus::StorageError;

match handles.writer.insert_metric(metric) {
    Ok(()) => println!("Inserted"),
    Err(StorageError::ChannelSend) => eprintln!("Writer actor not running"),
    Err(StorageError::Database(e)) => eprintln!("DuckDB error: {e}"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

## Architecture Notes

```text
┌──────────────────┐     MPSC Channel     ┌──────────────────┐
│  StorageWriter   │ ──────────────────▶  │    DbActor       │
│  StorageAdmin    │                      │  (Single Writer) │
│                  │                      │                  │
└──────────────────┘                      └────────┬─────────┘
                                                   │
                                                   ▼
┌──────────────────┐     r2d2 Pool        ┌──────────────────┐
│  MetricReader    │ ◀─────────────────   │    DuckDB        │
│  EventReader     │                      │    (File)        │
│  RawSqlReader    │                      │                  │
└──────────────────┘                      └──────────────────┘
```

- **Write path**: Commands sent via sync MPSC to dedicated writer thread (uses DuckDB Appender for high throughput)
- **Read path**: r2d2 connection pool for concurrent reads
- **Visibility**: Call `checkpoint()` after writes to ensure read visibility (DuckDB WAL)
