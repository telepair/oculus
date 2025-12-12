# Oculus Product Requirements Document (PRD)

| Metadata        | Value                                              |
| --------------- | -------------------------------------------------- |
| Project         | Oculus (Repository: telepair/oculus)               |
| Version         | v0.1.0 (Genesis / MVP)                             |
| Status          | Confirmed / Ready for Development                  |
| Last Updated    | 2025-12-12                                         |
| Core Philosophy | Value Monitoring, Single Binary, Zero Dependencies |

---

## 1. Executive Summary

Oculus is an out-of-the-box value monitoring software designed for individuals and small teams. Written in Rust with zero external dependencies. It provides unified monitoring for various numeric values including cryptocurrency prices, stock market data, prediction market odds, and network metrics.

**Core Value Proposition:**

- **Value Monitoring**: Track any numeric value with a unified interface
- **Zero Dependencies**: Single binary, runs anywhere
- **Target Audience**: Individuals and small teams

**Value Types Supported:**

| Category           | Examples                                             |
| ------------------ | ---------------------------------------------------- |
| Cryptocurrency     | BTC price, ETH price                                 |
| Stock Market       | TSLA, GOOGL, SPY, QQQ                                |
| Prediction Markets | Polymarket prices                                    |
| Market Indicators  | Fear Index, S&P Multiples, Altcoin Speculation Index |
| Network Metrics    | HTTP RTT, HTTP Status Code                           |
| Custom Values      | Any RESTful API endpoint                             |

**v0.1.0 Goals:**

- Validate the "Rust + DuckDB + HTMX" tech stack
- Build a zero-dependency single binary application
- Complete the data pipeline: Collection → Persistence → [Rule Engine] → Notifications/Actions
- Explicitly **out of scope** for v0.1.0: any user/account/role management.

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
│  │  Crypto Market  │   │  Stock Market   │   │ Prediction Mkt  │               │
│  │  - BTC/ETH      │   │  - TSLA/GOOGL   │   │  - Polymarket   │               │
│  │  - Fear Index   │   │  - SPY/QQQ      │   │  - Event Odds   │               │
│  │  - MVRV Z-Score │   │  - Index Data   │   │                 │               │
│  │  - Altcoin Idx  │   └────────┬────────┘   └────────┬────────┘               │
│  └────────┬────────┘            │                     │                         │
│           │                     │                     │                         │
│  ┌────────┴─────────────────────┴─────────────────────┴────────┐               │
│  │                                                              │               │
│  │  ┌─────────────────┐              ┌─────────────────┐       │               │
│  │  │  Network Probe  │              │  Generic API    │       │               │
│  │  │  - HTTP RTT     │              │  - RESTful API  │       │               │
│  │  │  - HTTP Code    │              │  - JSON Path    │       │               │
│  │  │  - TCP/Ping     │              │  - Custom Value │       │               │
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
│       │  - Derived values           │   │  - Custom aggregations      │         │
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
│         PRESENTATION LAYER           │  │       NOTIFICATION / ACTION LAYER      │
├──────────────────────────────────────┤  ├────────────────────────────────────────┤
│  ┌────────────┐   ┌────────────────┐ │  │  ┌─────────┐  ┌─────────┐  ┌────────┐ │
│  │    Web     │   │      API       │ │  │  │   Log   │  │  Email  │  │  Chat  │ │
│  │  - HTMX    │   │  - REST JSON   │ │  │  │ tracing │  │  SMTP   │  │  TG    │ │
│  │  - Charts  │   │  - Query       │ │  │  └─────────┘  └─────────┘  │  DC    │ │
│  │  - Tables  │   │  - WebSocket   │ │  │                            │ Slack  │ │
│  └────────────┘   └────────────────┘ │  │                            └────────┘ │
│           │               │          │  │  ┌─────────┐      ┌──────────────────┐│
│           └───────┬───────┘          │  │  │ Webhook │      │   HTTP POST      ││
│                   ▼                  │  │  │         │      │   Action         ││
│          ┌────────────────┐          │  │  └─────────┘      └──────────────────┘│
│          │     Axum       │          │  │              │                │       │
│          │  (Web Server)  │          │  │              └────────┬───────┘       │
│          └────────────────┘          │  │                       ▼               │
└──────────────────────────────────────┘  │              ┌────────────────┐       │
                                          │              │   Dispatcher   │       │
                                          │              └────────────────┘       │
                                          └────────────────────────────────────────┘
```

**Concurrency Model:**

- **Write**: Actor model (single writer via MPSC channel)
- **Read**: Connection pool (multiple concurrent readers)

### 2.3 Component Overview

| Layer            | Component         | Description                                             |
| ---------------- | ----------------- | ------------------------------------------------------- |
| **Collector**    | Crypto Market     | BTC/ETH prices, Fear & Greed Index, MVRV, Altcoin Index |
|                  | Stock Market      | Stock prices (TSLA, GOOGL), index data (SPY, QQQ)       |
|                  | Prediction Market | Polymarket odds and prices                              |
|                  | Network Probe     | HTTP RTT, HTTP status code, TCP connect, ICMP ping      |
|                  | Generic API       | RESTful API with JSON path extraction                   |
| **Storage**      | DuckDB            | Embedded OLAP database, single-file columnar storage    |
| **Rule Engine**  | Simple Rules      | YAML/TOML-based threshold and range checks              |
|                  | Complex Rules     | Raw SQL for advanced multi-metric analysis              |
|                  | Derived Values    | Calculate new values from existing metrics              |
| **Presentation** | Web UI            | HTMX-powered real-time dashboard                        |
|                  | API               | REST JSON endpoints + optional WebSocket                |
| **Notification** | Log               | Structured logging via `tracing` crate                  |
|                  | Email             | SMTP-based alerts                                       |
|                  | Chat              | Telegram, Discord, Slack integrations                   |
|                  | Webhook           | Generic webhook for custom integrations                 |
| **Action**       | HTTP POST         | Trigger external automation via HTTP POST               |

---

## 3. Data Architecture

**Database File:** `./oculus.db` (auto-created on startup)

### 3.1 Core Table: `metrics`

| Field    | Type        | Constraint                    | Description                                       |
| -------- | ----------- | ----------------------------- | ------------------------------------------------- |
| ts       | TIMESTAMPTZ | NOT NULL, Index               | Timestamp when data was generated (UTC)           |
| category | VARCHAR     | NOT NULL, default `'default'` | Logical group for aggregation/filters             |
| symbol   | VARCHAR     | NOT NULL                      | Unique metric identifier (e.g., `crypto.btc`)     |
| value    | DOUBLE      | NOT NULL                      | Numeric value for aggregation                     |
| tags     | JSON        | Nullable                      | Extended fields (e.g., `{"source": "coingecko"}`) |

**Query Capability:** Supports `WHERE tags->>'source' = 'coingecko'` syntax.

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

#### [Collect-01] Crypto Market Indicators

| Indicator          | Description                                | Output Symbol Example        |
| ------------------ | ------------------------------------------ | ---------------------------- |
| Asset Price        | Spot price from exchanges (BTC, ETH, etc.) | `crypto.price.btc_usd`       |
| Fear & Greed Index | Market sentiment indicator (0-100)         | `crypto.sentiment.fng`       |
| MVRV Z-Score       | Market Value to Realized Value ratio       | `crypto.onchain.mvrv`        |
| Altcoin Season Idx | Altcoin vs Bitcoin performance ratio       | `crypto.sentiment.altseason` |
| S&P Multiples      | S&P 500 valuation multiples                | `market.valuation.sp_pe`     |

- **Data Sources**: CoinGecko, Glassnode, Alternative.me, etc.
- **Rate Limiting**: Respect API quotas with exponential backoff

#### [Collect-02] Stock Market Indicators

| Indicator   | Description                    | Output Symbol Example |
| ----------- | ------------------------------ | --------------------- |
| Stock Price | Real-time/delayed quotes       | `stock.price.tsla`    |
| Index Data  | Major indices (S&P500, NASDAQ) | `stock.index.spy`     |
| ETF Data    | Major ETFs (QQQ, etc.)         | `stock.etf.qqq`       |

- **Data Sources**: Yahoo Finance, Alpha Vantage, etc.
- **Market Hours**: Respect trading hours, cache last value outside hours

#### [Collect-03] Prediction Market Indicators

| Indicator   | Description                              | Output Symbol Example   |
| ----------- | ---------------------------------------- | ----------------------- |
| Polymarket  | Event contract prices and odds           | `pred.poly.us_election` |
| Market Prob | Implied probability from contract prices | `pred.prob.event_xyz`   |

- **WebSocket**: Real-time price updates when available
- **REST Fallback**: Polling for markets without WebSocket support

#### [Collect-04] Network Probe

| Probe Type | Description                                | Output Symbol Example  |
| ---------- | ------------------------------------------ | ---------------------- |
| HTTP       | HTTP(S) response time (RTT), status code   | `net.http.api.example` |
| TCP        | TCP connect latency to host:port           | `net.tcp.redis:6379`   |
| Ping       | ICMP echo latency (ms) and packet loss (%) | `net.ping.google.com`  |

- **Interval**: Configurable per-probe (default 30s)
- **Timeout**: Per-probe timeout (default 5s)
- **Tags**: `{"probe": "http", "target": "https://api.example.com"}`

#### [Collect-05] Generic API Collector (RESTful)

| Feature   | Description                              | Example                        |
| --------- | ---------------------------------------- | ------------------------------ |
| URL       | Any RESTful API endpoint                 | `https://api.example.com/data` |
| Method    | HTTP method (GET, POST)                  | `GET`                          |
| JSON Path | Extract value using JSON path expression | `$.data.price`                 |
| Headers   | Custom HTTP headers                      | `Authorization: Bearer xxx`    |
| Interval  | Collection interval                      | `60s`                          |

```yaml
# Example: Generic API collector configuration
collectors:
  - type: api
    name: custom_index
    url: https://api.example.com/v1/index
    method: GET
    headers:
      Authorization: "Bearer ${API_TOKEN}"
    json_path: $.data.value
    interval: 60s
    tags:
      source: example_api
```

- **Use Cases**: Internal APIs, proprietary data sources, third-party services
- **Error Handling**: Graceful handling of network errors, invalid responses

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

  - name: api_slow
    condition: "net.http.api_example.rtt > 1000"
    for: 2m # Must persist for 2 minutes
    severity: warn
    message: "API response time exceeds 1s"
    channels: [slack, webhook]
    action:
      type: http_post
      url: https://automation.example.com/scale-up
```

| Feature   | Description                                      |
| --------- | ------------------------------------------------ |
| Threshold | Simple comparison: `>`, `<`, `==`, `!=`          |
| Duration  | `for: Xm` - condition must persist for X minutes |
| Debounce  | Prevent alert storms with cooldown period        |
| Channels  | Route to specific notification channels          |
| Action    | Trigger external action when rule fires          |

#### [Rule-02] Complex Rules (Raw SQL)

```sql
-- Example: Detect market volatility pattern
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

#### [Rule-03] Derived Values

```yaml
# Example: Calculate derived metrics
derived:
  - name: btc_eth_ratio
    formula: "crypto.price.btc_usd / crypto.price.eth_usd"
    interval: 60s

  - name: portfolio_value
    formula: "crypto.price.btc_usd * 0.5 + crypto.price.eth_usd * 10"
    interval: 60s
```

- **Use Cases**: Portfolio tracking, ratio analysis, custom indicators

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

- Category-based color coding (crypto=orange, stock=green, network=blue)
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

#### [Notify-04] Webhook

| Config     | Description                 |
| ---------- | --------------------------- |
| `url`      | Webhook endpoint URL        |
| `method`   | HTTP method (default: POST) |
| `headers`  | Custom HTTP headers         |
| `template` | JSON template for payload   |

```yaml
# Example: Webhook configuration
notifications:
  webhook:
    url: https://hooks.example.com/notify
    method: POST
    headers:
      Content-Type: application/json
      X-Custom-Header: value
    template: |
      {
        "event": "{{event_type}}",
        "severity": "{{severity}}",
        "message": "{{message}}",
        "timestamp": "{{timestamp}}"
      }
```

### 4.5 Action Layer

#### [Action-01] HTTP POST Trigger

Execute HTTP POST requests when rules fire to trigger external automation.

| Config    | Description           |
| --------- | --------------------- |
| `url`     | Action endpoint URL   |
| `method`  | HTTP method (POST)    |
| `headers` | Custom HTTP headers   |
| `body`    | Request body template |

```yaml
# Example: HTTP POST action
rules:
  - name: auto_scale
    condition: "net.http.api.rtt > 2000"
    for: 5m
    severity: critical
    channels: [telegram, log]
    action:
      type: http_post
      url: https://automation.example.com/scale
      headers:
        Authorization: "Bearer ${AUTOMATION_TOKEN}"
      body: |
        {
          "action": "scale_up",
          "service": "api",
          "reason": "High response time"
        }
```

- **Use Cases**: Auto-scaling, incident response, pipeline triggers
- **Retry Policy**: Configurable retry with exponential backoff

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
