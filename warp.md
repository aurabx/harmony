# warp.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

Follow the AI Agent Constitution at .aiassistant/rules/constitution.md at all times.

Repository: harmony-proxy - Rust-based proxy/gateway for data meshes (with first-class healthcare support)

## What This Project Does

Harmony is a proxy/gateway that handles, transforms and proxies data between systems. It provides secure communication with support for HTTP/JSON, FHIR, JMIX, DICOM, and DICOMweb protocols, featuring configurable middleware, authentication (JWT), audit logging, and WireGuard networking.

**Key Features:**
- Multi-protocol support: HTTP passthrough, FHIR, JMIX, DICOM, DICOMweb (QIDO-RS/WADO-RS endpoints)
- Multi-content-type: Automatic parsing of JSON, XML, CSV, form data, multipart, and binary content
- Hot configuration reload: zero-downtime updates for routes/middleware/backends, selective adapter restart for network changes
- Configurable routing with groups, endpoints, backends, and middleware
- JWT and basic authentication
- Request/response transformation pipeline
- AES-256-GCM encryption with ephemeral keys
- Envelope-based data exchange format
- Management API for monitoring and administration
- Security: XXE prevention, CSV formula injection mitigation, configurable size limits

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
- [docs/content-types.md](docs/content-types.md) - **Multi-content-type support**: JSON, XML, CSV, form data, multipart, binary
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
   - **Runbeam Cloud**: Optional cloud integration controlled by `[runbeam]` section
     - Default: `enabled = false` (all cloud features disabled)
     - When enabled, requires valid OAuth token for `/admin/authorize` endpoint

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

## Hot Configuration Reload

**Status**: Available since v0.4.0

Harmony supports hot-reloading configuration changes without requiring a full application restart.

### Quick Reference

**Zero-downtime changes** (instant):
- Middleware configuration (transforms, auth rules)
- Route definitions (endpoints, backends, pipelines)
- Backend URLs, timeouts
- Logging settings
- Storage configuration

**Adapter restart required** (~1-2s interruption for affected networks):
- Network bind addresses/ports
- Adding/removing networks
- WireGuard settings
- Protocol-specific settings

### How It Works

1. File watcher monitors config file (200ms debounce)
2. Changes validated before applying
3. Diff computed to classify change impact
4. For zero-downtime changes: atomic config swap via `ArcSwap`
5. For network changes: selective adapter restart (only affected networks)
6. Invalid configs rejected, old config retained

### Automatic Reload

Enabled by default - just edit and save your config file:

```bash
cargo run -- --config examples/test-config.toml
# Edit config file in another window
# Changes detected and applied automatically
```

### Monitoring

Watch logs for reload events:
```
📡 Watching config file for changes: config.toml
✓ Config reloaded successfully
  Zero-downtime changes: ["middleware", "endpoints"]
```

For adapter restarts:
```
✓ Config reloaded successfully
  Networks restarted: ["default"]
```

### Testing

Integration tests verify hot-reload behavior:
```bash
cargo test --test config_reload_integration
```

Tests cover:
- Zero-downtime middleware changes
- Adapter restart on port change
- Invalid config rejection
- Network add/remove
- Adapter registry lifecycle

See [docs/config-reload.md](docs/config-reload.md) for full architecture details.

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

## Path Filter Middleware

The path filter middleware uses explicit allow/deny rules with first-match-wins evaluation:

**Configuration:**
```toml
[middleware.my_filter]
type = "path_filter"
[middleware.my_filter.options]
rules = [
  { allow = "/api/public/{*path}" },  # Allow public API (catch-all under /api/public)
  { deny = "/api/{*path}" },           # Deny other API paths
  { allow = "/health" },               # Allow health check
  { deny = "/{*rest}" }                # Catch-all: deny everything else
]
```

**Evaluation Rules:**
- Rules are processed in order from first to last
- First matching rule determines outcome (allow or deny)
- Allow rule: request continues to backend
- Deny rule: middleware returns `PathDenied`; the HTTP adapter maps this to HTTP 404 and stops processing
- No match: implicit deny (middleware returns `PathDenied`, mapped to 404)

**Pattern Syntax:**
- Exact paths: `/users`
- Wildcards: `/api/{*path}` (catches all paths under /api/)
- Parameters: `/users/{id}`
- Multiple segments: `/api/{version}/users/{id}`

See docs/middleware.md for complete documentation.

**Important Notes:**
- Rule order matters - more specific patterns should come before broader patterns
- No backward compatibility with old string-based format
- Use `{*name}` syntax for catch-all wildcards (matchit requirement)

## Configuration Structure

Harmony uses a layered configuration approach with the following key sections:

### Basic Configuration

```toml
[proxy]
id = "harmony-proxy"
pipelines_path = "pipelines"
transforms_path = "transforms"
jwks_cache_duration_hours = 24  # JWKS cache for JWT validation (1-168 hours)

[logging]
log_level = "error"  # trace, debug, info, warn, error
log_to_file = true
log_file_path = "./tmp/harmony.log"

[runbeam]
enabled = false  # Set to true to enable cloud integration
# cloud_api_base_url = "https://api.runbeam.cloud"  # Optional, defaults to this URL
# poll_interval_secs = 30  # Optional, polling interval (5-3600 seconds, default 30)

[management]
enabled = true
base_path = "admin"
network = "management"  # Network to bind management API
```

### Runbeam Cloud Integration

The `[runbeam]` section controls integration with Runbeam Cloud for configuration management:

- **enabled**: Controls all cloud features. When false (default), no cloud operations are performed.
- **cloud_api_base_url**: API endpoint for Runbeam Cloud (defaults to https://api.runbeam.cloud)
- **poll_interval_secs**: How often to poll for configuration updates (5-3600 seconds, default 30)

When cloud integration is disabled:
- No token loading from environment or storage
- No cloud polling at startup  
- `/admin/authorize` endpoint returns 403 Forbidden
- Gateway runs in standalone mode

When enabled, the gateway will:
- Check for existing machine tokens at startup
- Poll Runbeam Cloud for configuration changes
- Apply cloud-sourced configuration updates automatically
- Automatically download referenced transform specifications before applying configs
- Require authorization via `/admin/authorize` endpoint

### Logging Configuration

```
[logging]
log_level = "debug"  # Moved here
log_to_file = true
log_file_path = "./tmp/harmony.log"
```

Environment variable `RUST_LOG` overrides `logging.log_level` if set.

## DICOM Services (SCU/SCP)

Harmony supports both DICOM Service Class User (SCU) and Service Class Provider (SCP) operations through separate service types.

### DICOM SCU (Backend - Outgoing Requests)

Use `dicom_scu` service for backends that make outgoing requests to remote PACS systems.

**Configuration Example:**
```toml
[backends.remote_pacs]
service = "dicom_scu"

[backends.remote_pacs.options]
aet = "REMOTE_PACS"              # Remote AE Title (required)
host = "pacs.example.com"        # Remote host (required)
port = 4242                       # Remote port (required)
local_aet = "HARMONY_SCU"        # Local AE Title (default: HARMONY_SCU)
dimse_retrieve_mode = "get"      # "get" or "move" (default: get)
use_tls = false                   # Enable TLS (default: false)
```

**Supported Operations:**
- `C-ECHO` - Test connectivity
- `C-FIND` - Query for studies/series/images
- `C-MOVE` - Request dataset transfer to destination AET
- `C-GET` - Direct dataset retrieval (recommended)

### DICOM SCP (Endpoint - Incoming Requests)

Use `dicom_scp` service for endpoints that receive incoming DICOM requests.

**Automatic Adapter Selection**: When you configure a `dicom_scp` endpoint in a pipeline, Harmony automatically starts the `DimseAdapter` for that network. You don't need to explicitly configure protocol adapters—they're determined based on the services used in your pipelines.

**Configuration Example:**
```toml
[endpoints.dicom_listener]
service = "dicom_scp"

[endpoints.dicom_listener.options]
local_aet = "HARMONY_SCP"        # Local AE Title (required, 1-16 chars)
bind_addr = "0.0.0.0"            # Bind address (default: 0.0.0.0)
port = 11112                      # Listen port (default: 11112)
enable_echo = true                # Enable C-ECHO (default: true)
enable_find = true                # Enable C-FIND (default: false)
enable_move = true                # Enable C-MOVE (default: false)
enable_get = true                 # Enable C-GET (default: false)
storage_dir = "./data/dicom"     # Storage directory (optional)
```

**Supported Operations:**
- `C-ECHO` - Connectivity test (always enabled by default)
- `C-FIND` - Query operations (must enable explicitly)
- `C-MOVE` - Transfer requests (must enable explicitly)
- `C-GET` - Direct retrieval (must enable explicitly)
- `C-STORE` - Store incoming datasets (planned)

### Complete Pipeline Example

**Scenario:** Receive DICOM queries via SCP, proxy to remote PACS via SCU

```toml
[network.dicom_network]
enable_wireguard = false
interface = "wg0"

[network.dicom_network.http]
bind_address = "127.0.0.1"
bind_port = 8080

[pipelines.dicom_bridge]
description = "DICOM SCP to SCU bridge"
networks = ["dicom_network"]
endpoints = ["dicom_listener"]
backends = ["remote_pacs"]
middleware = []  # Add auth/transforms as needed

[endpoints.dicom_listener]
service = "dicom_scp"

[endpoints.dicom_listener.options]
local_aet = "BRIDGE_SCP"
port = 11112
enable_echo = true
enable_find = true
enable_get = true

[backends.remote_pacs]
service = "dicom_scu"

[backends.remote_pacs.options]
aet = "PACS_AET"
host = "pacs.hospital.org"
port = 4242
local_aet = "BRIDGE_SCU"

[services.dicom_scp]
module = ""

[services.dicom_scu]
module = ""
```

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
