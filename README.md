# Oculus

Unified Telemetry • Single Binary • No-Build Web

[![Crates.io](https://img.shields.io/crates/v/oculus.svg)](https://crates.io/crates/oculus)
[![Documentation](https://docs.rs/oculus/badge.svg)](https://docs.rs/oculus)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Oculus is a lightweight, high-performance monitoring and observability system that bridges the gap between traditional infrastructure monitoring (like Prometheus) and financial market tracking (like TradingView).

> ⚠️ **Status**: v0.1.0 (Genesis / MVP) - Under active development  
> 🛑 **Scope Note**: v0.1.0 is single-user; no user/account/role management.

## Features

- 🚀 **Single Binary** - Zero external dependencies, runs anywhere
- 📊 **Unified Metrics** - Network probes, crypto, stocks, and prediction markets in one place
- ⚡ **High Performance** - Rust + DuckDB for low latency and minimal resource usage
- 🌐 **No-Build Web UI** - HTMX-powered dashboard, no JavaScript build step
- 🔔 **Smart Alerts** - Rule engine with simple YAML configs and raw SQL support
- 📬 **Multi-Channel Notifications** - Log, Email, Telegram, Discord, Slack

> 📖 See [PRD](docs/PRD.md) for detailed functional requirements.

## Architecture

```text
Collectors  →  MPSC Channel  →  DuckDB  →  Rule Engine  →  Presentation/Notifications
    │                              │              │                    │
    ├─ Network Probes              └─ Single      ├─ Simple Rules      ├─ Web UI (HTMX)
    ├─ Crypto Markets                  Writer        (YAML/TOML)       ├─ REST API
    ├─ Stock Markets                              ├─ Complex Rules     ├─ Telegram
    ├─ Prediction Markets                            (Raw SQL)         ├─ Discord
    └─ Custom Collectors                                               └─ Slack
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

# Build debug binary
make build

# Run with default config
make run

# Or build optimized release
make release
```

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
# Example: config.yaml
collectors:
  - type: network
    probe: ping
    target: 8.8.8.8
    interval: 30s

  - type: crypto
    asset: btc_usd
    source: coingecko
    interval: 60s

rules:
  - name: btc_price_alert
    condition: "crypto.price.btc_usd > 100000"
    severity: info
    channels: [telegram, log]
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
├── docs/
│   ├── PRD.md           # Product Requirements Document
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

> v0.1.0 MVP milestone tasks based on [PRD](docs/PRD.md)

### Collector Layer

- [ ] Core collector trait and MPSC channel pipeline
- [ ] Network Probe: Ping, TCP, HTTP health checks
- [ ] Crypto Market: Asset price, Fear & Greed Index, MVRV
- [ ] Stock Market: Stock price, index data
- [ ] Prediction Market: Polymarket integration
- [ ] Custom collector plugin interface

### Storage Layer

- [x] DuckDB integration with single-writer actor model
- [x] `metrics` table schema and migrations
- [x] `events` table for alerts/audit
- [x] Data retention and cleanup policy (7-day window)

### Rule Engine

- [ ] Simple rules: YAML/TOML-based threshold alerts
- [ ] Complex rules: Raw SQL with scheduled execution
- [ ] Event emitter for rule triggers

### Presentation Layer

- [ ] Axum web server setup
- [ ] HTMX dashboard homepage (`GET /`)
- [ ] Real-time metrics partial (`GET /partials/metrics`)
- [ ] REST API endpoints (`/api/metrics`, `/api/query`, etc.)
- [ ] Askama templates with Tailwind CSS (bundled)

### Notification Layer

- [ ] Log output via `tracing` crate
- [ ] Email notifications (SMTP)
- [ ] Telegram bot integration
- [ ] Discord webhook support
- [ ] Slack webhook support

### Infrastructure

- [ ] Configuration file parsing (YAML)
- [ ] CLI argument handling
- [ ] Health probes (`/healthz`, `/readyz`)
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

**Oculus** - _See everything, miss nothing._
