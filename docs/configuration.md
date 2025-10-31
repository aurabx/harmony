# Configuration

**Last Updated**: 2025-01-18 (Phase 6)

## Overview

Harmony uses a two-layer configuration model:
- **Top-level config**: Networks, storage, logging, service registrations
- **Pipeline files**: Endpoints, middleware, backends, and routing rules

**Protocol adapters** (HTTP, DIMSE, etc.) are automatically spawned based on pipeline configurations. See [adapters.md](adapters.md) for details.

Top-level config (examples/default/config.toml)
- [proxy]: service identity, logging level, and store_dir
- [network.<name>]: network interfaces and options
  - [network.<name>.http]: bind_address and bind_port
- pipelines_path: directory containing pipeline files
- transforms_path: directory for custom transforms (if used)
- [logging]: file logging options
- [services.*]: built-in or custom service types
- [middleware_types.*]: built-in or custom middleware types

Pipeline files (examples/default/pipelines/*.toml)
- `[pipelines.<name>]`: binds a set of endpoints, middleware, and backends to one or more networks
  - `networks`: list of network names from the top-level config
  - `endpoints`: list of endpoint names defined in this file
  - `middleware`: ordered list of middleware names (applied in sequence)
  - `backends`: list of backend names defined in this file
- `[middleware.<name>]`: middleware instances and their config
- `[endpoints.<name>]`: endpoint instances with service type and options
- `[backends.<name>]`: backend instances with service type and target configuration
- `[targets.<name>]`: concrete destinations that a backend selects from
- `[endpoint_types.*]`, `[service_types.*]`: register built-in or custom types

**Protocol adapters** are spawned automatically:
- **HttpAdapter**: Started for pipelines with HTTP/FHIR/JMIX/DICOMweb endpoints
- **DimseAdapter**: Started for pipelines with DICOM DIMSE endpoints
- See `src/lib.rs::run()` for orchestration logic

Validation expectations
- Networks must define valid HTTP bind_address and non-zero bind_port
- Each pipeline should reference at least one network, endpoint, and backend
- Unknown middleware names cause validation failure
- Middleware config is parsed by the middleware modules themselves

Examples
- Minimal passthrough: examples/default/pipelines/default.toml
- FHIR passthrough: examples/default/pipelines/fhir.toml
- FHIR to DICOM flow: examples/default/pipelines/fhir-dicom.toml

## Hot Configuration Reload

**Status**: Available since v0.4.0

Harmony supports hot-reloading configuration changes without requiring a full application restart. Changes are automatically detected and applied based on their impact.

### How It Works

The config file watcher monitors your configuration file for changes with a 200ms debounce. When changes are detected:

1. **Validation**: New config is validated before applying
2. **Diff Computation**: Changes are classified into categories
3. **Apply Strategy**:
   - **Zero-downtime changes**: Atomic config swap (instant)
   - **Adapter restarts**: Selective restart of affected networks only
4. **Logging**: Reload results logged with details

### Change Classification

#### Zero-Downtime Changes (Instant)

These changes are picked up on the next request with no service interruption:

- **Middleware configuration**: Transform specs, auth rules, custom middleware options
- **Route definitions**: Groups, endpoints, backends, path patterns
- **Backend configuration**: Target URLs, timeouts, retry policies
- **Logging settings**: Log levels, file paths, output formats
- **Storage configuration**: Backend type, paths, connection strings

#### Adapter Restart Required (Brief Interruption)

These changes require restarting specific protocol adapters (~1-2 second interruption for affected networks only):

- **Network topology**: Bind addresses, bind ports
- **Adding/removing networks**: New adapters started or old ones stopped
- **WireGuard configuration**: Interface names, peer settings
- **Protocol-specific settings**: TLS certificates, connection pool sizes

### Usage

#### Automatic Reload (Default)

File watching is enabled by default when you run Harmony:

```bash
cargo run -- --config config/config.toml
# or
./harmony --config config/config.toml
```

Simply edit `config/config.toml` and save - changes will be detected and applied automatically.

#### Reload Behavior Examples

**Example 1: Zero-Downtime Change**
```toml
# Add middleware to existing config
[middleware.my_transform]
type = "transform"

[middleware.my_transform.options]
spec_path = "transforms/my_spec.json"
```

Result: Atomic config swap, next request uses new middleware. No downtime.

**Example 2: Adapter Restart**
```toml
# Change HTTP port
[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8081  # Changed from 8080
```

Result: HTTP adapter for "default" network restarted on new port. DIMSE adapter unaffected. Brief interruption (~1-2s) for HTTP requests on "default" network only.

**Example 3: Network Addition**
```toml
# Add new network
[network.secondary]
interface = "eth1"

[network.secondary.http]
bind_address = "0.0.0.0"
bind_port = 8082
```

Result: New adapters started for "secondary" network. Existing "default" network unaffected.

### Error Handling

#### Invalid Configuration

If the new configuration is invalid:
- Validation errors are logged with details
- **Old configuration is retained** - application continues running
- No service interruption

Example log output:
```
❌ Config reload failed: Configuration validation failed
  Reason: Network 'default' has invalid bind port
```

#### Partial Reload Failure

If adapter restart fails:
- Error logged with network name
- Old adapter continues running if possible
- Other networks unaffected

### Monitoring Reloads

Watch the logs for reload events:

```
📡 Watching config file for changes: config/config.toml
✓ Config reloaded successfully
  Zero-downtime changes: ["middleware", "endpoints"]
```

Or for adapter restarts:
```
✓ Config reloaded successfully
  Networks restarted: ["default"]
  Zero-downtime changes: ["backends"]
```

### Best Practices

1. **Test config changes locally** before deploying to production
2. **Use zero-downtime changes** when possible (middleware, routes, backends)
3. **Schedule network topology changes** during low-traffic periods
4. **Monitor logs** after config changes to verify successful reload
5. **Keep backups** of previous config versions for quick rollback
6. **Staged rollout**: Change non-critical settings first, then network topology

### Limitations

- **In-flight requests**: Requests active during adapter restart may fail
- **No connection draining**: Adapters shut down immediately (future enhancement)
- **No automatic rollback**: Failed partial reloads keep old config but may leave inconsistent state
- **No audit trail**: Config changes not logged/tracked (future enhancement)

### Troubleshooting

**Config changes not detected?**
- Verify file watcher is active (check logs for "📡 Watching config file")
- Ensure 200ms debounce period has elapsed
- Check file permissions

**Reload failed?**
- Check logs for validation errors
- Verify config syntax with `cargo run -- --config config/config.toml --validate` (future feature)
- Ensure referenced files (transforms, certs) exist

**Adapters not restarting?**
- Verify network name matches config
- Check for port conflicts
- Review adapter-specific logs

For more details on the hot-reload architecture, see [docs/config-reload.md](config-reload.md).

---

Notes
- Prefer ./tmp for temporary files rather than /tmp
- For realistic JWT auth configuration, see docs/middleware.md
