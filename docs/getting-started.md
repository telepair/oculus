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

| Option          | Env Variable         | Default               | Description         |
| --------------- | -------------------- | --------------------- | ------------------- |
| `-c, --config`  | `OCULUS_CONFIG`      | `configs/config.yaml` | Config file path    |
| `--server-bind` | `OCULUS_SERVER_BIND` | `0.0.0.0`             | Server bind address |
| `--server-port` | `OCULUS_SERVER_PORT` | `8080`                | Server port         |
| `--db-path`     | `OCULUS_DB_PATH`     | `data/oculus.db`      | Database path       |

**Priority**: CLI > Environment > Config File

## Configuration

Oculus uses a main config file and a directory of collector configs:

```yaml
# configs/config.example.yaml
server:
  bind: 0.0.0.0
  port: 8080

database:
  driver: sqlite
  dsn: data/oculus.db
  channel_capacity: 10000

# Path to directory with collector config files
collector_include: configs/collectors/
```

### Collector Configuration

Collectors are defined in separate YAML files under `configs/collectors/`:

```yaml
# configs/collectors/tcp.yaml
tcp:
  - name: google-dns
    host: 8.8.8.8
    port: 53
    enabled: true
    group: production
    interval: 5s
    timeout: 5s
    tags:
      env: production

# configs/collectors/ping.yaml
ping:
  - name: cloudflare-ping
    host: 1.1.1.1
    enabled: true
    group: production
    interval: 30s
    timeout: 3s

# configs/collectors/http.yaml
http:
  - name: api-health
    url: https://api.example.com/health
    enabled: true
    group: production
    method: GET
    interval: 30s
    timeout: 10s
    headers:
      Authorization: "Bearer ${API_TOKEN:-default}"
    success_conditions:
      status_codes: [200, 201]
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
│   ├── config.example.yaml
│   └── collectors/   # Collector config files
├── docs/             # Documentation
└── Makefile          # Build automation
```
