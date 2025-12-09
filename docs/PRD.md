# Oculus Product Requirements Document (PRD)

| Metadata        | Value                                          |
| --------------- | ---------------------------------------------- |
| Project         | Oculus (Repository: telepair/oculus)           |
| Version         | v0.1.0 (Genesis / MVP)                         |
| Status          | Confirmed / Ready for Development              |
| Last Updated    | 2025-12-09                                     |
| Core Philosophy | Unified Telemetry, Single Binary, No-Build Web |

---

## 1. Executive Summary

Oculus is a lightweight, high-performance monitoring and observability
system. It bridges the gap between traditional infrastructure monitoring
(like Prometheus) and financial market tracking (like TradingView).

**v0.1.0 Goals:**

- Validate the "Rust + DuckDB + HTMX" tech stack
- Build a zero-dependency single binary application
- Complete the data pipeline:
  Collection → Persistence → [Rule Engine] → Real-time Web Display
- Explicitly **out of scope** for v0.1.0: any user/account/role management;
  single-tenant with one static token only.

---

## 2. System Architecture

### 2.1 Tech Stack

| Layer         | Technology             | Rationale                                       |
| ------------- | ---------------------- | ----------------------------------------------- |
| Language      | Rust (Edition 2024)    | Memory safety, zero GC pause, ideal for daemons |
| Async Runtime | Tokio                  | High-concurrency I/O and task scheduling        |
| Web Framework | Axum                   | Lightweight, modular, Tokio-native              |
| Database      | DuckDB (Embedded)      | Single-file OLAP, columnar storage, rich SQL    |
| Frontend      | HTMX                   | AJAX via HTML attributes, no JS build required  |
| Templating    | Askama                 | Type-safe, compiles to Rust, SSR performance    |
| Styling       | Tailwind CSS (Bundled) | Prebuilt CSS shipped with binary (offline)      |

### 2.2 Logical Architecture

```text
┌─────────────────────────────────────────────────────────────────────────────────┐
│                              COLLECTOR LAYER                                     │
├─────────────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐   ┌─────────────────┐   ┌─────────────────┐               │
│  │  Network Probe  │   │  Crypto Market  │   │  Stock Market   │               │
│  │  - Ping         │   │  - Asset Price  │   │  - Stock Price  │               │
│  │  - TCP          │   │  - Fear & Greed │   │  - Index Data   │               │
│  │  - HTTP         │   │  - MVRV Z-Score │   │  - Sector Stats │               │
│  └────────┬────────┘   │  - Altcoin Idx  │   └────────┬────────┘               │
│           │            │  - MA 30/60     │            │                         │
│           │            └────────┬────────┘            │                         │
│           │                     │                     │                         │
│  ┌────────┴─────────────────────┴─────────────────────┴────────┐               │
│  │                                                              │               │
│  │  ┌─────────────────┐              ┌─────────────────┐       │               │
│  │  │ Prediction Mkt  │              │     Custom      │       │               │
│  │  │  - Polymarket   │              │  - User-defined │       │               │
│  │  │  - Odds/Prices  │              │  - Plugins      │       │               │
│  │  └────────┬────────┘              └────────┬────────┘       │               │
│  │           │                                │                 │               │
│  └───────────┴────────────────────────────────┴─────────────────┘               │
│                              │                                                   │
│                              ▼                                                   │
│                     ┌────────────────┐                                          │
│                     │  MPSC Channel  │                                          │
│                     └────────┬───────┘                                          │
└──────────────────────────────┼──────────────────────────────────────────────────┘
                               │
┌──────────────────────────────┼──────────────────────────────────────────────────┐
│                              ▼            STORAGE LAYER                          │
├─────────────────────────────────────────────────────────────────────────────────┤
│                     ┌────────────────┐                                          │
│                     │   DB Actor     │  (Single Writer)                         │
│                     └────────┬───────┘                                          │
│                              │                                                   │
│                              ▼                                                   │
│                     ┌────────────────┐                                          │
│                     │   oculus.db    │  DuckDB (Embedded OLAP)                  │
│                     └────────┬───────┘                                          │
└──────────────────────────────┼──────────────────────────────────────────────────┘
                               │
┌──────────────────────────────┼──────────────────────────────────────────────────┐
│                              ▼            RULE ENGINE                            │
├─────────────────────────────────────────────────────────────────────────────────┤
│       ┌─────────────────────────────┐   ┌─────────────────────────────┐         │
│       │      Simple Rules           │   │      Complex Rules          │         │
│       │  (YAML/TOML Configuration)  │   │      (Raw SQL Queries)      │         │
│       │  - Threshold alerts         │   │  - Multi-metric correlation │         │
│       │  - Range checks             │   │  - Time-series analysis     │         │
│       │  - Status monitors          │   │  - Custom aggregations      │         │
│       └─────────────┬───────────────┘   └─────────────┬───────────────┘         │
│                     │                                 │                          │
│                     └─────────────┬───────────────────┘                          │
│                                   ▼                                              │
│                          ┌────────────────┐                                      │
│                          │  Event Emitter │                                      │
│                          └────────┬───────┘                                      │
└───────────────────────────────────┼──────────────────────────────────────────────┘
                                    │
       ┌────────────────────────────┴────────────────────────────┐
       │                                                         │
       ▼                                                         ▼
┌──────────────────────────────────────┐  ┌────────────────────────────────────────┐
│         PRESENTATION LAYER           │  │           NOTIFICATION LAYER           │
├──────────────────────────────────────┤  ├────────────────────────────────────────┤
│  ┌────────────┐   ┌────────────────┐ │  │  ┌─────────┐  ┌─────────┐  ┌────────┐ │
│  │    Web     │   │      API       │ │  │  │   Log   │  │  Email  │  │  Chat  │ │
│  │  - HTMX    │   │  - REST JSON   │ │  │  │ tracing │  │  SMTP   │  │  TG    │ │
│  │  - Charts  │   │  - Query       │ │  │  └─────────┘  └─────────┘  │  DC    │ │
│  │  - Tables  │   │  - WebSocket   │ │  │                            │ Slack  │ │
│  └────────────┘   └────────────────┘ │  │                            └────────┘ │
│           │               │          │  │              │                │       │
│           └───────┬───────┘          │  │              └────────┬───────┘       │
│                   ▼                  │  │                       ▼               │
│          ┌────────────────┐          │  │              ┌────────────────┐       │
│          │     Axum       │          │  │              │   Dispatcher   │       │
│          │  (Web Server)  │          │  │              │                │       │
│          └────────────────┘          │  │              └────────────────┘       │
└──────────────────────────────────────┘  └────────────────────────────────────────┘
```

**Concurrency Model:**

- **Write**: Actor model (single writer via MPSC channel)
- **Read**: Connection pool (multiple concurrent readers)

### 2.3 Component Overview

| Layer            | Component         | Description                                           |
| ---------------- | ----------------- | ----------------------------------------------------- |
| **Collector**    | Network Probe     | ICMP ping, TCP connect, HTTP health checks            |
|                  | Crypto Market     | Asset prices, Fear & Greed Index, MVRV, Altcoin Index |
|                  | Stock Market      | Stock prices, index data, sector statistics           |
|                  | Prediction Market | Polymarket odds and prices                            |
|                  | Custom            | User-defined collectors via plugin interface          |
| **Storage**      | DuckDB            | Embedded OLAP database, single-file columnar storage  |
| **Rule Engine**  | Simple Rules      | YAML/TOML-based threshold and range checks            |
|                  | Complex Rules     | Raw SQL for advanced multi-metric analysis            |
| **Presentation** | Web UI            | HTMX-powered real-time dashboard                      |
|                  | API               | REST JSON endpoints + optional WebSocket              |
| **Notification** | Log               | Structured logging via `tracing` crate                |
|                  | Email             | SMTP-based alerts                                     |
|                  | Chat              | Telegram, Discord, Slack integrations                 |

---

## 3. Data Architecture

**Database File:** `./oculus.db` (auto-created on startup)

### 3.1 Core Table: `metrics`

| Field    | Type        | Constraint                    | Description                                   |
| -------- | ----------- | ----------------------------- | --------------------------------------------- |
| ts       | TIMESTAMPTZ | NOT NULL, Index               | Timestamp when data was generated (UTC)       |
| category | VARCHAR     | NOT NULL, default `'default'` | Logical group for aggregation/filters         |
| symbol   | VARCHAR     | NOT NULL                      | Unique metric identifier (e.g., `sim.random`) |
| value    | DOUBLE      | NOT NULL                      | Numeric value for aggregation                 |
| tags     | JSON        | Nullable                      | Extended fields (e.g., `{"region": "us"}`)    |

**Query Capability:** Supports `WHERE tags->>'region' = 'hk'` syntax.

### 3.2 Event Table: `events` (Optional)

| Field    | Type        | Constraint                                                       | Description                                         |
| -------- | ----------- | ---------------------------------------------------------------- | --------------------------------------------------- |
| id       | BIGINT      | Primary key, AUTO INCREMENT                                      | Event identifier                                    |
| ts       | TIMESTAMPTZ | NOT NULL, INDEX                                                  | Event timestamp (UTC)                               |
| source   | VARCHAR     | NOT NULL                                                         | Event origin: `rule.<name>`, `collector.<name>` ... |
| type     | VARCHAR     | NOT NULL, CHECK `type IN ('alert','error','system','audit')`     | Event nature/handling class                         |
| severity | VARCHAR     | NOT NULL, CHECK `severity IN ('info','warn','error','critical')` | Delivery priority / urgency                         |
| message  | VARCHAR     | NOT NULL                                                         | Short human-readable description                    |
| payload  | JSON        | Nullable                                                         | Context snapshot (e.g., request/response, config)   |

#### Data retention

- Default retention window: 7 days (configurable)
- Cleanup policy: run on startup and hourly
  `DELETE FROM metrics WHERE ts < now() - interval '7 days'`
- Compression: DuckDB automatic; optional daily `VACUUM`

---

## 4. Functional Requirements

### 4.1 Collectors

#### [Collect-01] Network Probe

| Probe Type | Description                                    | Output Symbol Example  |
| ---------- | ---------------------------------------------- | ---------------------- |
| Ping       | ICMP echo latency (ms) and packet loss (%)     | `net.ping.google.com`  |
| TCP        | TCP connect latency to host:port               | `net.tcp.redis:6379`   |
| HTTP       | HTTP(S) response time, status code, body match | `net.http.api.example` |

- **Interval**: Configurable per-probe (default 30s)
- **Timeout**: Per-probe timeout (default 5s)
- **Tags**: `{"probe": "ping", "target": "8.8.8.8"}`

#### [Collect-02] Crypto Market Indicators

| Indicator          | Description                                | Output Symbol Example        |
| ------------------ | ------------------------------------------ | ---------------------------- |
| Asset Price        | Spot price from exchanges (BTC, ETH, etc.) | `crypto.price.btc_usd`       |
| Fear & Greed Index | Market sentiment indicator (0-100)         | `crypto.sentiment.fng`       |
| MVRV Z-Score       | Market Value to Realized Value ratio       | `crypto.onchain.mvrv`        |
| Altcoin Season Idx | Altcoin vs Bitcoin performance ratio       | `crypto.sentiment.altseason` |
| Moving Averages    | MA30/MA60 for trend analysis               | `crypto.ma.btc_30d`          |

- **Data Sources**: CoinGecko, Glassnode, Alternative.me, etc.
- **Rate Limiting**: Respect API quotas with exponential backoff

#### [Collect-03] Stock Market Indicators

| Indicator    | Description                         | Output Symbol Example |
| ------------ | ----------------------------------- | --------------------- |
| Stock Price  | Real-time/delayed quotes            | `stock.price.aapl`    |
| Index Data   | Major indices (S&P500, NASDAQ, HSI) | `stock.index.spx`     |
| Sector Stats | Sector ETF performance              | `stock.sector.xlk`    |

- **Data Sources**: Yahoo Finance, Alpha Vantage, etc.
- **Market Hours**: Respect trading hours, cache last value outside hours

#### [Collect-04] Prediction Market Indicators

| Indicator   | Description                              | Output Symbol Example   |
| ----------- | ---------------------------------------- | ----------------------- |
| Polymarket  | Event contract prices and odds           | `pred.poly.us_election` |
| Market Prob | Implied probability from contract prices | `pred.prob.event_xyz`   |

- **WebSocket**: Real-time price updates when available
- **REST Fallback**: Polling for markets without WebSocket support

#### [Collect-05] Custom Collectors

- **Plugin Interface**: Trait-based collector definition
- **Configuration**: YAML/TOML for custom endpoint, interval, parsing rules
- **Use Cases**: Internal APIs, proprietary data sources, webhooks

### 4.2 Rule Engine

#### [Rule-01] Simple Rules (Configuration-based)

```yaml
# Example: rules.yaml
rules:
  - name: btc_price_alert
    condition: "crypto.price.btc_usd > 100000"
    severity: info
    message: "BTC crossed $100k!"
    channels: [telegram, log]

  - name: api_down
    condition: "net.http.api.example.status != 200"
    for: 2m # Must persist for 2 minutes
    severity: critical
    message: "API endpoint unreachable"
    channels: [email, slack, log]
```

| Feature   | Description                                      |
| --------- | ------------------------------------------------ |
| Threshold | Simple comparison: `>`, `<`, `==`, `!=`          |
| Duration  | `for: Xm` - condition must persist for X minutes |
| Debounce  | Prevent alert storms with cooldown period        |
| Channels  | Route to specific notification channels          |

#### [Rule-02] Complex Rules (Raw SQL)

```sql
-- Example: Detect whale accumulation pattern
SELECT
  symbol,
  AVG(value) as avg_price,
  MAX(value) - MIN(value) as volatility
FROM metrics
WHERE category = 'crypto.price'
  AND ts > now() - INTERVAL '1 hour'
GROUP BY symbol
HAVING volatility > avg_price * 0.05
```

| Feature          | Description                                        |
| ---------------- | -------------------------------------------------- |
| Full SQL Support | DuckDB SQL with window functions, CTEs, aggregates |
| Scheduled Exec   | Cron-like schedule (e.g., `*/5 * * * *`)           |
| Result Binding   | Map query results to alert templates               |
| Parameterization | Dynamic thresholds from config                     |

### 4.3 Presentation Layer

#### [Web-01] Dashboard Homepage

| Attribute | Value                                            |
| --------- | ------------------------------------------------ |
| Path      | `GET /`                                          |
| Logic     | Query recent metrics → Render dashboard          |
| Style     | Dark mode, responsive, category-based panels     |
| Assets    | Local bundled Tailwind CSS, HTMX script (vendor) |

#### [Web-02] Real-time Data Stream

| Attribute | Value                                         |
| --------- | --------------------------------------------- |
| Path      | `GET /partials/metrics`                       |
| Logic     | Read-only connection → Query → Render partial |
| Trigger   | `hx-trigger="every 2s"` or WebSocket push     |

**Visual Enhancement:**

- Category-based color coding (network=blue, crypto=orange, stock=green)
- Trend indicators (↑↓) based on recent change
- Sparkline mini-charts for quick trend visualization

#### [Web-03] REST API

| Endpoint            | Method | Description                 |
| ------------------- | ------ | --------------------------- |
| `/api/metrics`      | GET    | List metrics with filters   |
| `/api/metrics/{id}` | GET    | Get specific metric details |
| `/api/query`        | POST   | Execute read-only SQL query |
| `/api/rules`        | GET    | List configured rules       |
| `/api/rules/{name}` | GET    | Get rule status and history |
| `/api/alerts`       | GET    | List recent alerts          |

**Safety Constraints:**

- Query timeout: 5 seconds
- Result limit: 1000 rows maximum
- Read-only mode prevents destructive operations
- Authentication: token header required (static secret for MVP)
- Rate limiting: per-IP token bucket (10 req/s, burst 20)

### 4.4 Notification Layer

#### [Notify-01] Log Output

- **Backend**: `tracing` crate with structured JSON logs
- **Levels**: `TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`
- **Rotation**: Daily rotation with configurable retention

#### [Notify-02] Email (SMTP)

| Config         | Description                              |
| -------------- | ---------------------------------------- |
| `smtp_host`    | SMTP server hostname                     |
| `smtp_port`    | Port (465/587)                           |
| `smtp_user`    | Authentication username                  |
| `smtp_pass`    | Authentication password (from env/vault) |
| `from_address` | Sender email address                     |
| `to_addresses` | Recipient list                           |

- **Template**: HTML email with alert details, metric snapshot
- **Batching**: Group multiple alerts within 1-minute window

#### [Notify-03] Chat Integrations

| Platform | Integration Method           | Config Key           |
| -------- | ---------------------------- | -------------------- |
| Telegram | Bot API (HTTP POST)          | `telegram_bot_token` |
| Discord  | Webhook URL                  | `discord_webhook`    |
| Slack    | Incoming Webhook / Bot Token | `slack_webhook`      |

- **Message Format**: Rich formatting with severity colors, inline charts
- **Mention Support**: `@channel`, `@user` for critical alerts
- **Rate Limiting**: Respect platform API limits

---

## 5. Non-Functional Requirements

### 5.1 Performance & Resources

| Metric              | Target         | Validation                                  |
| ------------------- | -------------- | ------------------------------------------- |
| Startup Time        | < 100ms        | `hyperfine --warmup 3 './oculus --help'`    |
| Memory (Idle)       | < 50MB         | `ps` sampling after 1 min idle              |
| Binary Size         | < 20MB         | `strip && cargo build --release`            |
| Tailwind Delivery   | Bundled CSS    | Prebuilt file served locally, no network    |
| Cold Start Coverage | Route smoke OK | Health check on `/` and `/partials/metrics` |

### 5.2 Security

- **SQL Injection Prevention**: All internal queries use parameter binding (`?`)
- **Open Query Protection**: Physical write isolation via `AccessMode::ReadOnly`
- **Resource Protection**: Query timeout and result limits on `/api/query`
- **Offline Assets**: Tailwind CSS prebuilt locally; no external fetch at runtime

### 5.3 Observability

- Integrate `tracing` crate
- Configurable log levels via `RUST_LOG=info|debug`
- Structured logging for: DB connections, write failures, web requests
- Export counters: writer queue length, insert failures,
  query latency p95/p99, active queries
- Health probes: `/healthz` (process up), `/readyz` (DB ready + writer alive)
