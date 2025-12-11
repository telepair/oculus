# Getting Started

Build, configure, and run Oculus.

## Prerequisites

- **Rust** 2024 edition (`rustup default nightly`)
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
  # Interval-based collector
  - type: network.tcp
    name: redis-probe
    target: 127.0.0.1:6379
    interval: 30s
    timeout: 5s
    tags:
      env: production

  # Cron-based collector
  - type: network.tcp
    name: ssh-probe
    target: 127.0.0.1:22
    cron: "0 */5 * * * *"
    timeout: 10s
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
