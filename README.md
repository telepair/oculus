# Oculus

Value Monitoring • Single Binary • Zero Dependencies

[![Crates.io](https://img.shields.io/crates/v/oculus.svg)](https://crates.io/crates/oculus)
[![Documentation](https://docs.rs/oculus/badge.svg)](https://docs.rs/oculus)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Oculus is an out-of-the-box value monitoring software designed for individuals and small teams. Written in Rust, single binary, zero external dependencies.

> - ⚠️ **Status**: v0.1.1 - Under active development
> - 🛑 **Scope Note**: v0.1.x is single-user; no user/account/role management.

## Features

- 🚀 **Zero Dependencies** - Single binary, runs anywhere
- 📊 **Unified Value Monitoring** - Crypto, stocks, prediction markets, network metrics in one place
- ⚡ **High Performance** - Rust + DuckDB for low latency and minimal resource usage
- 🌐 **No-Build Web UI** - HTMX-powered dashboard, no JavaScript build step
- 🔔 **Rule Engine** - Calculate derived values, trigger alerts with YAML configs and raw SQL
- 📬 **Multi-Channel Notifications** - Log, Email, Telegram, Discord, Slack, Webhook
- 🎯 **Actions** - HTTP POST triggers for automation

> 📖 See [PRD](docs/PRD.md) for detailed functional requirements.

## Supported Value Types

| Category               | Examples                                 | Description                             |
| ---------------------- | ---------------------------------------- | --------------------------------------- |
| **Cryptocurrency**     | BTC, ETH                                 | Asset prices from exchanges             |
| **Stock Market**       | TSLA, GOOGL, SPY, QQQ                    | Stock and index prices                  |
| **Prediction Markets** | Polymarket                               | Event contract prices and odds          |
| **Market Indicators**  | Fear Index, S&P Multiples, Altcoin Index | Sentiment and valuation metrics         |
| **Network Metrics**    | HTTP, TCP, Ping                          | Response time, status codes, latency    |
| **Custom Values**      | RESTful API                              | Any numeric value via generic collector |

## Architecture

```text
Collectors  →  MPSC Channel  →  DuckDB  →  Rule Engine  →  Notifications/Actions
    │                              │              │                    │
    ├─ Crypto Prices               └─ Single      ├─ Simple Rules      ├─ Log
    ├─ Stock Prices                    Writer        (YAML/TOML)       ├─ Email
    ├─ Prediction Markets                         ├─ Complex Rules     ├─ Telegram
    ├─ Market Indicators                             (Raw SQL)         ├─ Discord
    ├─ Network Probes (HTTP/TCP/Ping)                                  ├─ Slack
    └─ Generic API Collector                                           ├─ Webhook
                                                                       └─ HTTP POST Action
```

> 📖 See [PRD](docs/PRD.md) for detailed logical architecture and component overview.

## Tech Stack

| Layer         | Technology   | Purpose                         |
| ------------- | ------------ | ------------------------------- |
| Language      | Rust 2024    | Memory safety, zero GC pause    |
| Async Runtime | Tokio        | High-concurrency I/O            |
| Web Framework | Axum         | Lightweight, Tokio-native       |
| Database      | DuckDB       | Embedded OLAP, columnar storage |
| Frontend      | HTMX         | AJAX via HTML attributes        |
| Templating    | Askama       | Type-safe, compiled templates   |
| Styling       | Tailwind CSS | Bundled, offline-first          |

## Quick Start

### Prerequisites

- Rust toolchain (Edition 2024)
- Optional: Tailwind CSS CLI for style customization

### Build & Run

```bash
# Clone the repository
git clone https://github.com/telepair/oculus.git
cd oculus

# Build full distribution (CSS + release binary)
make release

# Run with default config
./target/release/oculus

# Run with custom options
./target/release/oculus --config configs/config.yaml --server-port 9090
```

> 📖 See [Getting Started](docs/getting-started.md) for detailed configuration options.

### Development

```bash
# Full CI pipeline: format → lint → check → test → build
make all

# Individual commands
make fmt        # Format code
make lint       # Run clippy + markdownlint
make test       # Run tests
make doc-open   # Build and view documentation
```

## Configuration

Oculus uses a YAML configuration file for collectors and rules:

```yaml
# Example: configs/config.yaml
collectors:
  # Crypto price collector
  - type: crypto
    asset: btc_usd
    source: coingecko
    interval: 60s

  # Stock price collector
  - type: stock
    symbol: TSLA
    source: yahoo
    interval: 5m

  # Polymarket collector
  - type: polymarket
    market_id: some-market-id
    interval: 30s

  # Network probe
  - type: network.http
    target: https://api.example.com/health
    interval: 30s

  # Generic API collector (RESTful)
  - type: api
    name: custom_metric
    url: https://api.example.com/v1/data
    method: GET
    json_path: $.data.value
    interval: 60s

rules:
  - name: btc_price_alert
    condition: "crypto.price.btc_usd > 100000"
    severity: info
    channels: [telegram, log]

  - name: api_slow_response
    condition: "network.http.api_example.rtt > 1000"
    severity: warn
    channels: [slack, webhook]
    action:
      type: http_post
      url: https://automation.example.com/trigger

notifications:
  telegram:
    bot_token: ${TELEGRAM_BOT_TOKEN}
    chat_id: ${TELEGRAM_CHAT_ID}

  webhook:
    url: https://hooks.example.com/notify

  slack:
    webhook_url: ${SLACK_WEBHOOK_URL}
```

## API Endpoints

| Endpoint       | Method | Description                 |
| -------------- | ------ | --------------------------- |
| `/`            | GET    | Dashboard homepage          |
| `/api/metrics` | GET    | List metrics with filters   |
| `/api/query`   | POST   | Execute read-only SQL query |
| `/api/rules`   | GET    | List configured rules       |
| `/api/alerts`  | GET    | List recent alerts          |
| `/healthz`     | GET    | Health check (process up)   |
| `/readyz`      | GET    | Readiness check (DB ready)  |

## Project Structure

```text
oculus/
├── src/
│   ├── lib.rs           # Library: shared core functionality
│   └── main.rs          # Binary: runs complete system
├── templates/           # Askama templates (compiled into binary)
│   ├── dashboard.html
│   └── static/css/
├── configs/             # Configuration examples
├── docs/
│   ├── PRD.md           # Product Requirements Document
│   ├── getting-started.md # Build & Run Guide
│   ├── lib.md           # Library Integration Guide
│   └── schema.md        # Database Schema Reference
├── Cargo.toml           # Rust dependencies
├── Makefile             # Build automation
└── LICENSE              # MIT License
```

### Library vs Binary

| Target | Path          | Purpose                                                          |
| ------ | ------------- | ---------------------------------------------------------------- |
| `lib`  | `src/lib.rs`  | Exports reusable modules (collector, storage, rule engine, etc.) |
| `bin`  | `src/main.rs` | Entry point for running the complete Oculus system               |

**Use as a library:**

```rust
use oculus::{Collector, Storage, RuleEngine};
```

**Run as a binary:**

```bash
cargo run --bin oculus
```

## Performance Targets

| Metric        | Target  |
| ------------- | ------- |
| Startup Time  | < 100ms |
| Memory (Idle) | < 50MB  |
| Binary Size   | < 20MB  |

## TODO

> v0.1.1 milestone tasks based on [PRD](docs/PRD.md)

### Collector Layer

- [x] Core collector trait and MPSC channel pipeline
- [ ] Crypto Market: Asset price (BTC, ETH), Fear & Greed Index, MVRV
- [ ] Stock Market: Stock price, index data (SPY, QQQ)
- [ ] Prediction Market: Polymarket integration
- [x] Network Probe: TCP port probe
- [x] Network Probe: ICMP ping probe
- [x] Network Probe: HTTP RTT and status code checks
- [ ] Generic API Collector: RESTful API with JSON path extraction

### Storage Layer

- [x] DuckDB integration with single-writer actor model
- [x] `metrics` table schema and migrations
- [x] `events` table for alerts/audit
- [x] Data retention and cleanup policy (configurable window)

### Rule Engine

- [ ] Simple rules: YAML/TOML-based threshold alerts
- [ ] Complex rules: Raw SQL with scheduled execution
- [ ] Derived value calculation
- [ ] Event emitter for rule triggers

### Presentation Layer

- [x] Axum web server setup
- [x] HTMX dashboard homepage (`GET /`)
- [x] Real-time metrics partial (`GET /api/metrics`)
- [x] Real-time events partial (`GET /api/events`)
- [x] Metrics stats endpoint (`GET /api/metrics/stats`)
- [ ] Raw SQL query endpoint (`POST /api/query`)
- [x] Askama templates with Tailwind CSS (bundled)

### Notification Layer

- [x] Log output via `tracing` crate
- [ ] Email notifications (SMTP)
- [ ] Telegram bot integration
- [ ] Discord webhook support
- [ ] Slack webhook support
- [ ] Generic webhook support

### Action Layer

- [ ] HTTP POST action trigger

### Infrastructure

- [x] Configuration file parsing (YAML)
- [x] CLI argument handling
- [x] Health probes (`/healthz`, `/readyz`)
- [ ] Performance validation (startup < 100ms, memory < 50MB)

## Contributing

Contributions are welcome! Please read the PRD in `docs/PRD.md` for architectural context before submitting changes.

1. Fork the repository
2. Create your feature branch (`git checkout -b feat/amazing-feature`)
3. Commit your changes (`git commit -s -m 'feat: add amazing feature'`)
4. Push to the branch (`git push origin feat/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

**Oculus** - _Monitor any value, miss nothing._
