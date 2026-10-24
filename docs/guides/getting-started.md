---
title: Getting Started
description: How to build and run Harmony for the first time.
status: active
updated: 2026-10-24
type: Guide
---

# Getting Started

**Status**: Alpha-quality software under active development. Some features are placeholders.

## Architecture Overview

Harmony uses a **protocol adapter architecture** where each protocol (HTTP, HTTP/3, DIMSE, HL7, etc.) has a dedicated adapter that feeds into a unified pipeline. See [adapters.md](../architecture/adapters.md) for details.

Prerequisites
- Rust (stable; repository currently targets recent stable toolchains)
- macOS or Linux
- DCMTK (required for DICOM C-MOVE on the SCU side and for the integration tests; C-ECHO, C-FIND, C-STORE and C-GET are native)
  - macOS (Homebrew): `brew install dcmtk`
  - Debian/Ubuntu: `sudo apt-get install dcmtk`
- Optional: WireGuard kernel module if you plan to use WireGuard features

Build
- Debug: cargo build
- Release: cargo build --release

Run
- Using the basic echo example:
  - cargo run -- --config examples/basic-echo/config.toml
- Each example under `examples/` has its own README with specific instructions
- The basic-echo example binds to 127.0.0.1:8080
- Access the echo endpoint at: http://localhost:8080/echo

Minimal pipeline example (HTTP -> Echo with dual networks)
```toml
[proxy]
id = "smoke-test"
log_level = "info"

[storage]
backend = "filesystem"
path = "./tmp"

# Management network for management API only
[network.management]
enable_wireguard = false
interface = "wg0"
[network.management.http]
bind_address = "127.0.0.1"
bind_port = 9090

# External network for client traffic
[network.external]
enable_wireguard = false
interface = "wg0"
[network.external.http]
bind_address = "0.0.0.0"
bind_port = 8080

# Enable management API (explicitly specify which network to use)
[management]
enabled = true
base_path = "admin"
network = "management"

[pipelines.core]
description = "HTTP->Echo smoke pipeline"
networks = ["external"]
endpoints = ["smoke_http"]
backends = ["echo_backend"]
middleware = ["middleware.passthru"]

[endpoints.smoke_http]
service = "http"
[endpoints.smoke_http.options]
path_prefix = "/smoke"

[backends.echo_backend]
service = "echo"
[backends.echo_backend.options]
path_prefix = "/echo-back"

[services.http]
module = ""
[services.echo]
module = ""

[middleware_types.passthru]
module = ""
```

Drive it locally (no server binding required in tests):
- Use the router builder in tests to call routes via oneshot (see tests/smoke_http_echo.rs for examples)

Conventions
- Temporary files: prefer ./tmp within the working directory over /tmp
- Logging: use RUST_LOG=harmony=debug,info for local debugging

## Environment Variables

Local development needs no environment variables. To see more log output, set `RUST_LOG`:

```bash
export RUST_LOG=harmony=debug,info
cargo run -- --config path/to/config.toml
```

`RUST_LOG` overrides `logging.log_level`, which defaults to `error`. For every variable Harmony reads, see [Environment Variables](../configuration/environment.md). For secret handling in production, see [Managing Secrets](../security/secrets.md).

Next steps
- Read [adapters.md](../architecture/adapters.md) to understand protocol adapters (HTTP, DIMSE, and how to add new protocols)
- Read [Configuration](../configuration/configuration-file.md) for config structure and pipeline files
- See [middleware.md](../configuration/middleware.md) for auth and transforms (including real JWT verification)
- See [router.md](../architecture/router.md) for pipeline execution flow
- See [testing.md](testing.md) for running tests
