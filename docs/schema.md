# Database Schema

Oculus uses DuckDB for embedded OLAP storage. This document describes the schema.

## Tables

### metrics

Time-series numeric data points.

| Column | Type | Constraint | Description |
|--------|------|------------|-------------|
| ts | BIGINT | NOT NULL | Timestamp (microseconds since Unix epoch) |
| category | VARCHAR | NOT NULL | Logical grouping (e.g., "crypto", "network") |
| symbol | VARCHAR | NOT NULL | Unique identifier (e.g., "crypto.price.btc_usd") |
| value | DOUBLE | NOT NULL | Numeric value |
| tags | VARCHAR | NULL | JSON-encoded extended attributes |
| success | BOOLEAN | NOT NULL DEFAULT TRUE | Whether collection was successful |
| duration_ms | BIGINT | NOT NULL DEFAULT 0 | Collection duration in milliseconds |

**Note**: Timestamps are stored as BIGINT (microseconds) instead of TIMESTAMPTZ for reliable cross-connection queries in DuckDB.

### events

Structured event records for alerts, errors, and audit logs.

| Column | Type | Constraint | Description |
|--------|------|------------|-------------|
| id | BIGINT | PRIMARY KEY | Auto-increment via `events_id_seq` |
| ts | BIGINT | NOT NULL | Timestamp (microseconds since Unix epoch) |
| source | VARCHAR | NOT NULL | Event origin (e.g., "rule.btc_alert") |
| type | event_type | NOT NULL | Event classification |
| severity | event_severity | NOT NULL | Priority level |
| message | VARCHAR | NOT NULL | Human-readable description |
| payload | VARCHAR | NULL | JSON-encoded context snapshot |

## Custom Types

### event_type (ENUM)

```sql
CREATE TYPE event_type AS ENUM ('alert', 'error', 'system', 'audit');
```

| Value | Description |
|-------|-------------|
| `alert` | Rule-triggered notification |
| `error` | Collector or system error |
| `system` | Internal system event (startup, shutdown) |
| `audit` | Audit trail entry |

### event_severity (ENUM)

```sql
CREATE TYPE event_severity AS ENUM ('debug', 'info', 'warn', 'error', 'critical');
```

| Value | Description |
|-------|-------------|
| `debug` | Verbose diagnostic info |
| `info` | Normal operation |
| `warn` | Potential issue |
| `error` | Error requiring investigation |
| `critical` | Severe failure requiring immediate action |

## DDL Reference

```sql
-- Metrics table
CREATE TABLE IF NOT EXISTS metrics (
    ts          BIGINT      NOT NULL,
    category    VARCHAR     NOT NULL,
    symbol      VARCHAR     NOT NULL,
    value       DOUBLE      NOT NULL,
    tags        VARCHAR,
    success     BOOLEAN     NOT NULL DEFAULT TRUE,
    duration_ms BIGINT      NOT NULL DEFAULT 0
);

-- Event types
CREATE TYPE IF NOT EXISTS event_type AS ENUM ('alert', 'error', 'system', 'audit');
CREATE TYPE IF NOT EXISTS event_severity AS ENUM ('debug', 'info', 'warn', 'error', 'critical');

-- Events table
CREATE SEQUENCE IF NOT EXISTS events_id_seq;
CREATE TABLE IF NOT EXISTS events (
    id        BIGINT      PRIMARY KEY DEFAULT NEXTVAL('events_id_seq'),
    ts        BIGINT      NOT NULL,
    source    VARCHAR     NOT NULL,
    type      event_type   NOT NULL,
    severity  event_severity NOT NULL,
    message   VARCHAR     NOT NULL,
    payload   VARCHAR
);
```

## Query Examples

### Recent metrics by category

```sql
SELECT ts, symbol, value
FROM metrics
WHERE category = 'crypto'
  AND ts > (EXTRACT(EPOCH FROM NOW()) * 1000000) - 3600000000  -- last hour
ORDER BY ts DESC
LIMIT 100;
```

### Events by severity

```sql
SELECT ts, source, message
FROM events
WHERE severity IN ('error', 'critical')
ORDER BY ts DESC;
```

### Metrics with JSON tags

```sql
SELECT symbol, value, tags
FROM metrics
WHERE tags IS NOT NULL
  AND json_extract_string(tags, '$.region') = 'us';
```

## Data Retention

Default retention: **7 days** (configurable)

Cleanup runs on startup and hourly:

```sql
DELETE FROM metrics WHERE ts < (EXTRACT(EPOCH FROM NOW()) * 1000000) - 604800000000;
DELETE FROM events WHERE ts < (EXTRACT(EPOCH FROM NOW()) * 1000000) - 604800000000;
```
