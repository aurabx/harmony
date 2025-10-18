# warp.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

Repository: harmony-proxy - Rust-based proxy/gateway for data meshes (with first-class healthcare support)

## What This Project Does

Harmony is a proxy/gateway that handles, transforms and proxies data between systems. It provides secure communication with support for HTTP/JSON, FHIR, JMIX, DICOM, and DICOMweb protocols, featuring configurable middleware, authentication (JWT), audit logging, and WireGuard networking.

**Key Features:**
- Multi-protocol support: HTTP passthrough, FHIR, JMIX, DICOM, DICOMweb (QIDO-RS/WADO-RS endpoints)
- Configurable routing with groups, endpoints, backends, and middleware
- JWT and basic authentication
- Request/response transformation pipeline
- AES-256-GCM encryption with ephemeral keys
- Envelope-based data exchange format
- Management API for monitoring and administration

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

# Lint with clippy (do not run unless requested - this messes up diffs)
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
- [docs/router.md](docs/router.md) - Pipeline architecture and request flow (Protocol Adapter → PipelineExecutor → Protocol Adapter)
- [docs/adapters.md](docs/adapters.md) - Protocol adapter guide (HTTP, DIMSE, future protocols)
- [docs/envelope.md](docs/envelope.md) - Core Envelope struct for data exchange
- [docs/endpoints.md](docs/endpoints.md) - Endpoint types (HTTP, FHIR, JMIX, DICOMweb)
- [docs/backends.md](docs/backends.md) - Backend types and target communication
- [docs/management-api.md](docs/management-api.md) - Management API for monitoring and administration

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
- **Middleware**: Names must be recognized (`jwt_auth`, `auth_sidecar`, `aurabox_connect`, `transform`)
- **WireGuard**: If `enable_wireguard=true`, `interface` must be non-empty

## Known Pitfalls

- Tests may fail against current validator - update test fixtures to include required groups/middleware configs
- Unknown middleware names cause immediate failure - extend configuration if adding new middleware
- Transform middleware requires valid JOLT specification files in JSON format
- For WireGuard networks, `interface` field is mandatory when `enable_wireguard=true`
- Default binary config path: `/etc/harmony/harmony-config.toml`

## Development Conventions

- **Error Handling**: Use structured `ConfigError` enum (see `src/config/mod.rs`)
- **Code Style**: rustfmt (default) + clippy linting required
- **Logging**: tracing with env-filter; use `RUST_LOG=harmony=debug,info` locally
- **Output Directory**: Use `./tmp` directory for temporary files (not system `/tmp`)
- **Dynamic Loading**: libloading supports custom endpoints/middleware (see `examples/custom_endpoint`)
- Items are only ready for production use if they are fully tested and contain no bugs.
- Clippy is not to be run as part of patches unless specifically requested
- Try not to mix concerns. 
- Don't write tests to accept failure to make a failing implementation pass. Keep the test failing till the implementation is fixed. Alternatively, mark the test as skipped.
- If a piece of code seems poorly architected or doesn't do what you might expect it to, prompt the user.

## Change Management and PR Hygiene

- Do not build PRs for commits unless specifically requested
- Keep changes narrowly scoped. Do not mix unrelated work (e.g., storage refactors vs. Clippy/lint/format changes) in the same PR.
- If you need to apply broad formatting or lint fixes, submit them as a separate PR from any functional changes.
- When a large refactor is necessary, split into clearly labeled commits (e.g., "storage: introduce backend abstraction" vs. "lint: clippy fixes, no logic changes").
- Avoid touching files outside the feature’s scope unless strictly required for compilation.
- Prefer incremental PRs over one large change; this improves reviewability and reduces risk.

Incident log:
- 2025-10-08: Mixed a storage refactor with widespread Clippy/test cleanups. This made it hard to review the storage changes. Policy updated above—never mix jobs in a single PR.

## Monorepo Context

This proxy is part of the larger Runbeam ecosystem:
- Works with JMIX schema files (configurable path, typically `../jmix`)
- Integrates with other Runbeam components for data exchange across verticals (healthcare is a primary focus)
- Uses shared `/samples` directory across implementations
- Compatible with Rust CLI tools that may consume its output

For troubleshooting configuration issues, always check that `examples/test-config.toml` works as a baseline, then adapt your configuration to match its structure.

## Transform Middleware

The transform middleware uses [Fluvio JOLT](https://github.com/infinyon/fluvio-jolt) to perform JSON-to-JSON transformations on request/response data.

### Configuration

```toml
[middleware.my_transform]
type = "transform"
[middleware.my_transform.options]
spec_path = "path/to/jolt_spec.json"
apply = "both"  # "left", "right", or "both" (default)
fail_on_error = true  # true (default) or false
```

**Field Descriptions:**
- `spec_path`: Path to the JOLT specification file (JSON format). Relative paths are resolved from the config directory.
- `apply`: When to apply the transform - "left" (request to backend), "right" (response from backend), or "both" (default)
- `fail_on_error`: Whether to fail the request on transformation errors (true) or log and continue (false)

### JOLT Specification Example

Example transformation from patient data to FHIR-like structure:

**Input JSON:**
```json
{
  "PatientID": "12345",
  "PatientName": "John Doe",
  "StudyInstanceUID": "1.2.3.4.5.6",
  "StudyDate": "2024-01-15"
}
```

**JOLT Spec (`samples/jolt/patient_to_fhir.json`):**
```json
[
  {
    "operation": "shift",
    "spec": {
      "PatientID": "resource.identifier[0].value",
      "PatientName": "resource.name[0].family",
      "StudyInstanceUID": "resource.extension[0].valueString",
      "StudyDate": "resource.extension[1].valueDate"
    }
  },
  {
    "operation": "default",
    "spec": {
      "resourceType": "Patient",
      "resource": {
        "identifier": [{
          "system": "http://example.com/patient-id"
        }],
        "name": [{
          "use": "usual"
        }]
      }
    }
  }
]
```

**Output JSON:**
```json
{
  "resourceType": "Patient",
  "resource": {
    "identifier": [{
      "system": "http://example.com/patient-id",
      "value": "12345"
    }],
    "name": [{
      "use": "usual",
      "family": "John Doe"
    }],
    "extension": [
      {
        "url": "http://example.com/study-uid",
        "valueString": "1.2.3.4.5.6"
      },
      {
        "url": "http://example.com/study-date",
        "valueDate": "2024-01-15"
      }
    ]
  }
}
```

### Pre-Transform Snapshot

The transform middleware automatically preserves the original `normalized_data` in the `normalized_snapshot` field before applying any transformations. This allows other middleware or debugging tools to access the pre-transform state.

### JOLT Operations Supported

- **shift**: Copy data from input to output with path transformations
- **default**: Apply default values where data is missing
- **remove**: Remove fields from the output
- **wildcards**: Use `*` and `&` for dynamic field matching

See the [Fluvio JOLT documentation](https://github.com/infinyon/fluvio-jolt) for complete specification details.
