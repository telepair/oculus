# Getting Started

Build, configure, and run Oculus.

## Prerequisites

- **Rust** 2024 edition
- **Tailwind CSS CLI** (auto-downloaded on first build)

## Build

```bash
# Full build: frontend CSS + backend binary (release mode)
make release

# Development build (debug mode, no CSS minification)
make build
```

The `make release` command produces a single standalone binary at `target/release/oculus`.

## Run

```bash
# Using config file (default: configs/config.yaml)
./target/release/oculus

# Custom config file
./target/release/oculus --config /path/to/config.yaml

# Override via CLI
./target/release/oculus --server-port 9090 --db-path /data/oculus.db

# Override via environment variables
OCULUS_SERVER_PORT=9090 OCULUS_DB_PATH=/data/oculus.db ./target/release/oculus
```

### CLI Options

| Option           | Env Variable          | Default               | Description             |
| ---------------- | --------------------- | --------------------- | ----------------------- |
| `-c, --config`   | `OCULUS_CONFIG`       | `configs/config.yaml` | Config file path        |
| `--server-bind`  | `OCULUS_SERVER_BIND`  | `0.0.0.0`             | Server bind address     |
| `--server-port`  | `OCULUS_SERVER_PORT`  | `8080`                | Server port             |
| `--db-path`      | `OCULUS_DB_PATH`      | `data/oculus.db`      | Database path           |
| `--db-pool-size` | `OCULUS_DB_POOL_SIZE` | `4`                   | DB connection pool size |

**Priority**: CLI > Environment > Config File

## Configuration

```yaml
# configs/config.yaml
server:
  bind: 0.0.0.0
  port: 8080

database:
  path: data/oculus.db
  pool_size: 4
  channel_capacity: 1000

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
    market_id: us-election-2024
    interval: 30s

  # Network probes (HTTP, TCP, Ping)
  - type: network.http
    name: api-health
    target: https://api.example.com/health
    interval: 30s
    timeout: 5s

  - type: network.tcp
    name: redis-probe
    target: 127.0.0.1:6379
    interval: 30s
    timeout: 5s

  - type: network.ping
    name: gateway-probe
    target: 8.8.8.8
    interval: 60s
    timeout: 3s

  # Generic API collector (for any RESTful API)
  - type: api
    name: fear_greed_index
    url: https://api.alternative.me/fng/
    method: GET
    json_path: $.data[0].value
    interval: 300s
    tags:
      source: alternative.me

rules:
  - name: btc_price_alert
    condition: "crypto.price.btc_usd > 100000"
    severity: info
    message: "BTC crossed $100k!"
    channels: [telegram, log]

  - name: api_slow_response
    condition: "network.http.api_health.rtt > 1000"
    severity: warn
    message: "API response time exceeds 1s"
    channels: [slack, webhook]
    action:
      type: http_post
      url: https://automation.example.com/alert

notifications:
  telegram:
    bot_token: ${TELEGRAM_BOT_TOKEN}
    chat_id: ${TELEGRAM_CHAT_ID}

  webhook:
    url: https://hooks.example.com/notify

  slack:
    webhook_url: ${SLACK_WEBHOOK_URL}

  email:
    smtp_host: smtp.example.com
    smtp_port: 587
    smtp_user: ${SMTP_USER}
    smtp_pass: ${SMTP_PASS}
    from_address: oculus@example.com
    to_addresses:
      - alerts@example.com
```

## Development

```bash
make all      # Full CI: fmt → lint → check → test → build
make fmt      # Format code
make lint     # Clippy + markdownlint
make test     # Run tests
make doc-open # View Rust docs
```

## Project Structure

```text
oculus/
├── src/
│   ├── lib.rs        # Library crate
│   └── main.rs       # Binary entry point
├── templates/        # Askama templates (compiled into binary)
│   ├── dashboard.html
│   └── static/css/   # Tailwind CSS
├── configs/          # Configuration examples
├── docs/             # Documentation
└── Makefile          # Build automation
```
