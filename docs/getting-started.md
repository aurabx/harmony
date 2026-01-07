# Getting Started

**Last Updated**: 2025-11-30

**Status**: Alpha-quality software under active development. Some features are placeholders.

## Architecture Overview

Harmony uses a **protocol adapter architecture** where each protocol (HTTP, HTTP/3, DIMSE, HL7, etc.) has a dedicated adapter that feeds into a unified pipeline. See [adapters.md](adapters.md) for details.

Prerequisites
- Rust (stable; repository currently targets recent stable toolchains)
- macOS or Linux
- DCMTK (required for DICOM SCU operations: C-GET and C-MOVE; optional for persistent C-STORE SCP)
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

Harmony uses environment variables for runtime configuration and security settings. These are optional for local development but recommended for production deployments.

### Quick Reference
**RUST_LOG** - Logging verbosity (optional, default: `info`)
- Example: `RUST_LOG=harmony=debug,info`
- Controls tracing output; use `debug` for development, `info` for production

**RUNBEAM_ENCRYPTION_KEY** - Token storage encryption key (optional for local dev, recommended for containers)
- Base64-encoded age X25519 key
- Example: Generated via `age-keygen | base64 | tr -d '\n'`
- In containers, ensures tokens survive restarts; without it, tokens are lost on restart

**RUNBEAM_JWT_SECRET** - JWT validation secret for Runbeam Cloud (required for production cloud integration)
- String (recommend 32+ character random value)
- Example: Generated via `openssl rand -base64 32`
- Must match secret configured in Runbeam Cloud


### Local Development Example

```bash
# Set logging level for detailed output
export RUST_LOG=harmony=debug,info

# Run with example configuration
cargo run -- --config examples/basic-echo/config.toml
```

### Production Container Example

```bash
# Generate encryption key for persistent token storage
export RUNBEAM_ENCRYPTION_KEY=$(age-keygen | base64 | tr -d '\n')

# Set JWT secret (must match Runbeam Cloud)
export RUNBEAM_JWT_SECRET="your-production-secret"

# Set production logging level
export RUST_LOG=harmony=info

# Run Harmony
cargo run --release -- --config /etc/harmony/config.toml
```

**Notes**:
- **RUNBEAM_ENCRYPTION_KEY**: In containers, this ensures tokens survive restarts. Without it, tokens are lost when container restarts. See [Security Documentation](security.md#runbeam_encryption_key) for platform-specific generation commands.
- **RUNBEAM_JWT_SECRET**: Required for Runbeam Cloud integration. Falls back to insecure development default with warning.
- **RUST_LOG**: Controls tracing output. Use `debug` for development, `info` for production.

For comprehensive environment variable documentation, security best practices, and production deployment examples, see [Security Documentation](security.md#environment-variables).

Next steps
- Read [adapters.md](adapters.md) to understand protocol adapters (HTTP, DIMSE, and how to add new protocols)
- Read [configuration.md](configuration.md) for config structure and pipeline files
- See [middleware.md](middleware.md) for auth and transforms (including real JWT verification)
- See [router.md](router.md) for pipeline execution flow
- See [testing.md](testing.md) for running tests
