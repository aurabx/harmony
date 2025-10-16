# Getting Started

Status: alpha-quality software under active development. Some features are placeholders.

Prerequisites
- Rust (stable; repository currently targets recent stable toolchains)
- macOS or Linux
- DCMTK (required if you use DICOM DIMSE features: endpoints/backends performing C-ECHO/C-FIND/C-MOVE/C-GET)
  - macOS (Homebrew): `brew install dcmtk`
  - Debian/Ubuntu: `sudo apt-get install dcmtk`
- Optional: WireGuard kernel module if you plan to use WireGuard features

Build
- Debug: cargo build
- Release: cargo build --release

Run
- Using the example configuration with dual networks:
  - cargo run -- --config examples/config/config.toml
- The config uses two networks: management (127.0.0.1:9090) for management API, external (0.0.0.0:8080) for client traffic
- Access management endpoints at: http://localhost:9090/admin/info
- Access client endpoints at: http://localhost:8080/

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

Next steps
- Read Configuration for how the top-level config and pipeline files fit together
- See Middleware for auth and transforms (including real JWT verification)
- See Testing for how to run fast, deterministic tests
