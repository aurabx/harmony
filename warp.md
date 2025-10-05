# warp.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

Repository: harmony-proxy - Rust-based proxy/gateway for healthcare systems

## What This Project Does

Harmony is a proxy/gateway that handles, transforms and proxies data between different healthcare systems. It provides secure communication with support for FHIR, JMIX, DICOM, and DICOMweb protocols, featuring configurable middleware, authentication (JWT), audit logging, and WireGuard networking.

**Key Features:**
- Multi-protocol support: HTTP passthrough, FHIR, JMIX, DICOM, DICOMweb
- Configurable routing with groups, endpoints, backends, and middleware
- JWT and basic authentication
- Request/response transformation pipeline
- AES-256-GCM encryption with ephemeral keys
- Envelope-based data exchange format

## Prerequisites

- Rust 1.87+ (edition 2021)
- Tokio runtime with full feature set
- Key dependencies: axum 0.8.x, tower 0.5.x, tracing + tracing-subscriber
- Optional: WireGuard kernel module (if using WireGuard features)

## Essential Commands

```bash
# Build (debug)
cargo build

# Build (release)
cargo build --release

# Run main service with test config
cargo run -- --config examples/test-config.toml

# Run with alternate config
cargo run -- --config <path/to/config.toml>

# Run all tests
cargo test

# Run focused test file
cargo test --test config_validation

# Run tests with logging
RUST_LOG=debug cargo test -- --nocapture

# Format code
cargo fmt --all

# Lint with clippy
cargo clippy --all-targets -- -D warnings
```

## Documentation Index

### Project Guidelines and Rules
- [.aiassistant/rules/ai.md](.aiassistant/rules/ai.md) - AI assistant context loading rules
- [.junie/guidelines.md](.junie/guidelines.md) - **Essential reading**: Build system, test strategy, validator behavior, development conventions, and known pitfalls

### Documentation
- [docs/getting-started.md](docs/getting-started.md) - Build, run, and local conventions
- [docs/configuration.md](docs/configuration.md) - Top-level and pipeline configuration
- [docs/middleware.md](docs/middleware.md) - Authentication (JWT, Basic) and transforms
- [docs/testing.md](docs/testing.md) - Testing strategy and commands
- [docs/security.md](docs/security.md) - Security guidance and best practices
- [docs/system-description.md](docs/system-description.md) - High-level system overview and Runbeam architecture
- [docs/router.md](docs/router.md) - Router behavior and request flow (Endpoint → Middleware → Service → Backend)
- [docs/envelope.md](docs/envelope.md) - Core Envelope struct for data exchange
- [docs/endpoints.md](docs/endpoints.md) - Endpoint types (HTTP, FHIR, JMIX, DICOMweb)
- [docs/backends.md](docs/backends.md) - Backend types and target communication

### Other
- [readme.md](readme.md) - Project overview and links

## Quick Start Development

1. **Local Development Setup**:
   ```bash
   # Use the test configuration as baseline
   cargo run -- --config examples/test-config.toml
   ```

2. **Configuration Requirements** (from .junie/guidelines.md):
   - At least one group must be defined with valid network references
   - Groups must reference existing endpoints/backends/middleware
   - Unknown middleware names cause immediate validation failure
   - Use only: `jwt_auth`, `auth_sidecar`, `aurabox_connect` unless extending config

3. **Directory Structure**:
   ```
   src/
   ├── lib.rs              # Library crate
   ├── bin/                # Binaries (main service)
   ├── config/             # Configuration validation
   └── backends/dicom/     # DICOM-specific validation
   examples/
   ├── test-config.toml    # Working test configuration
   └── custom_endpoint/    # Example custom endpoint crate
   tests/
   └── config_validation.rs # Integration tests
   ```

## Testing Strategy

- **Test-Driven Development**: Preferred approach for iterations
- **Current Test Status**: Some tests in `tests/config_validation.rs` may fail due to stricter validator requirements
- **Test Data**: Use configuration strings or files under `tests/data/` for hermetic tests
- **Sample Directory**: `/samples` directory available for test data

### Test Commands
```bash
# Full test suite
cargo test

# Single test file
cargo test --test config_validation

# With logging enabled
RUST_LOG=harmony=debug,info cargo test -- --nocapture
```

## Configuration Validation

The validator enforces strict requirements:

- **Networks**: Must not be empty; each network needs `http.bind_address` and non-zero `http.bind_port`
- **Groups**: At least one group; each group must reference existing networks
- **Middleware**: Names must be recognized (`jwt_auth`, `auth_sidecar`, `aurabox_connect`)
- **WireGuard**: If `enable_wireguard=true`, `interface` must be non-empty

## Known Pitfalls

- Tests may fail against current validator - update test fixtures to include required groups/middleware configs
- Unknown middleware names cause immediate failure - extend configuration if adding new middleware
- For WireGuard networks, `interface` field is mandatory when `enable_wireguard=true`
- Default binary config path: `/etc/harmony/harmony-config.toml`

## Development Conventions

- **Error Handling**: Use structured `ConfigError` enum (see `src/config/mod.rs`)
- **Code Style**: rustfmt (default) + clippy linting required
- **Logging**: tracing with env-filter; use `RUST_LOG=harmony=debug,info` locally
- **Output Directory**: Use `./tmp` directory for temporary files (not system `/tmp`)
- **Dynamic Loading**: libloading supports custom endpoints/middleware (see `examples/custom_endpoint`)

## Monorepo Context

This proxy is part of the larger Runbeam ecosystem:
- Works with JMIX schema files (configurable path, typically `../jmix`)
- Integrates with other Runbeam components for healthcare data exchange
- Uses shared `/samples` directory across implementations
- Compatible with Rust CLI tools that may consume its output

For troubleshooting configuration issues, always check that `examples/test-config.toml` works as a baseline, then adapt your configuration to match its structure.