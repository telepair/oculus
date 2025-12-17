# Library Integration Guide

This guide covers using Oculus as a Rust library for custom telemetry applications.

## Quick Start

Add to `Cargo.toml`:

```toml
[dependencies]
oculus = { git = "https://github.com/telepair/oculus.git" }
tokio = { version = "1", features = ["full"] }
chrono = "0.4"
```

## Basic Usage

```rust
use oculus::{
    StorageBuilder, MetricCategory, MetricSeries, MetricValue, StaticTags,
    MetricQuery,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build storage (spawns writer actor)
    let handles = StorageBuilder::new("sqlite:data/oculus.db?mode=rwc")
        .channel_capacity(1024)  // Writer command queue size
        .build()
        .await?;

    // Create a metric series (dimension data)
    let series = MetricSeries::new(
        MetricCategory::Custom,
        "my.metric",              // name
        "target-1",               // target
        StaticTags::new(),        // static tags for identity
        Some("My metric".into()), // description
    );
    let series_id = series.series_id;
    handles.writer.upsert_metric_series(series)?;

    // Insert metric values (time-series data)
    let value = MetricValue::new(series_id, 42.0, true);
    handles.writer.insert_metric_value(value)?;

    // Flush to ensure data is written
    handles.writer.flush()?;

    // Wait for actor to process
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Query via reader facade
    let results = handles.metric_reader.query(MetricQuery::default()).await?;
    println!("Found {} metrics", results.len());

    // Graceful shutdown
    handles.shutdown().await?;
    Ok(())
}
```

## API Overview

### Writer (MPSC Channel → Single Writer Task)

| Facade          | Methods                  | Description                                 |
| --------------- | ------------------------ | ------------------------------------------- |
| `StorageWriter` | `upsert_metric_series()` | Upsert metric series (deduped by series_id) |
|                 | `insert_metric_value()`  | Insert metric value (batched)               |
|                 | `insert_event()`         | Insert event (immediate)                    |
|                 | `flush()`                | Force flush buffered data                   |
|                 | `dropped_metrics()`      | Get count of dropped metrics                |

### Readers (Connection Pool)

| Facade         | Methods              | Description                          |
| -------------- | -------------------- | ------------------------------------ |
| `MetricReader` | `query(MetricQuery)` | Query metrics (series + values JOIN) |
|                | `stats(start, end)`  | Get aggregated statistics            |
| `EventReader`  | `query(EventQuery)`  | Query events with filters            |
| `RawSqlReader` | `execute(sql)`       | Execute raw SELECT queries           |

### Admin

| Facade         | Methods                   | Description              |
| -------------- | ------------------------- | ------------------------ |
| `StorageAdmin` | `cleanup_metric_values()` | Delete old metric values |
|                | `cleanup_events()`        | Delete old events        |
|                | `shutdown()`              | Graceful shutdown        |

## Data Types

### MetricSeries

Static dimension data identified by `series_id` (xxhash64 of category, name, target, static_tags).

```rust
let series = MetricSeries::new(
    MetricCategory::NetworkTcp,
    "latency",
    "127.0.0.1:6379",
    StaticTags::new(),
    Some("Redis latency".into()),
);
```

### MetricValue

Time-series data point linked to a series.

```rust
let value = MetricValue::new(series_id, 42.5, true)
    .with_unit("ms")
    .with_duration_ms(15)
    .with_tag("status_code", "200")
    .with_tag("path", "/api/v1");
```

### Event

Structured event with source, kind, severity.

```rust
use oculus::{Event, EventSource, EventKind, EventSeverity};

let event = Event::new(
    EventSource::System,
    EventKind::System,
    EventSeverity::Info,
    "Application started",
).with_payload("version", "1.0.0");

handles.writer.insert_event(event)?;
```

## Query Examples

```rust
use oculus::{MetricQuery, EventQuery, SortOrder, MetricCategory};
use chrono::{Utc, Duration};

// Query recent TCP metrics
let results = handles.metric_reader.query(MetricQuery {
    category: Some(MetricCategory::NetworkTcp),
    start: Some(Utc::now() - Duration::hours(1)),
    limit: Some(50),
    order: Some(SortOrder::Desc),
    ..Default::default()
}).await?;

// Raw SQL query
let rows = handles.raw_sql_reader.execute(
    "SELECT s.name, AVG(v.value) as avg
     FROM metric_values v
     JOIN metric_series s ON v.series_id = s.series_id
     GROUP BY s.name"
).await?;
```

## Error Handling

All operations return `Result<T, StorageError>`:

```rust
use oculus::StorageError;

match handles.writer.insert_metric_value(value) {
    Ok(()) => println!("Inserted"),
    Err(StorageError::ChannelSend) => eprintln!("Channel full or closed"),
    Err(StorageError::Database(e)) => eprintln!("SQLite error: {e}"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

## Architecture

```text
┌──────────────────┐     MPSC Channel     ┌──────────────────┐
│  StorageWriter   │ ──────────────────►  │    DbActor       │
│  StorageAdmin    │                      │  (Single Writer) │
│                  │                      │                  │
└──────────────────┘                      └────────┬─────────┘
                                                   │
                                                   ▼
┌──────────────────┐     Connection Pool  ┌──────────────────┐
│  MetricReader    │ ◀─────────────────   │     SQLite       │
│  EventReader     │                      │     (File)       │
│  RawSqlReader    │                      │                  │
└──────────────────┘                      └──────────────────┘
```

- **Write path**: Commands sent via async MPSC to dedicated writer task
- **Read path**: Connection pool for concurrent reads
- **Batching**: `MetricValue` inserts are buffered (500 items or 1 second)
