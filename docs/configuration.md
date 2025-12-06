# Configuration

**Last Updated**: 2025-11-30

## Environment Variable Substitution

Harmony supports environment variable substitution in all TOML configuration values. This allows you to externalize sensitive data, deployment-specific settings, and other configuration that varies across environments.

### Syntax

Use `$VARIABLE_NAME` to reference environment variables in any TOML value:

```toml
[proxy]
id = "$PROXY_ID"

[network.default.http]
bind_address = "$BIND_ADDRESS"
bind_port = $BIND_PORT

[backends.remote]
service = "http"

[backends.remote.options]
url = "https://$REMOTE_HOST:$REMOTE_PORT/api"
```

### Variable Names

Valid variable names must:
- Start with a letter or underscore
- Contain only alphanumeric characters and underscores
- Examples: `$VAR`, `$VAR_NAME`, `$_VAR123`
- Invalid: `$123VAR` (starts with number), `$MY-VAR` (contains hyphen)

### Behavior

**If variable is set**: Replaced with its value
```bash
export HOST=example.com
# $HOST in config becomes: example.com
```

**If variable is not set**: Replaced with empty string and warning logged
```toml
url = "https://$UNDEFINED_VAR/api"  # becomes: https:///api (warning logged)
```

**Escaped dollar signs**: Use `$$` to represent a literal `$` character
```toml
description = "Cost is $$100"  # becomes: Cost is $100
```

### Examples

#### Externalize Secrets

```toml
[backends.production]
service = "http"

[backends.production.options]
url = "https://api.example.com"
api_key = "$API_KEY"
```

Set at runtime:
```bash
export API_KEY="secret_key_xyz"
cargo run -- --config config/config.toml
```

#### Deployment-Specific Configuration

```toml
[network.default.http]
bind_address = "$BIND_ADDRESS"
bind_port = $BIND_PORT

[proxy]
id = "harmony-$ENVIRONMENT"
```

Run different environments:
```bash
# Development
export BIND_ADDRESS=127.0.0.1 BIND_PORT=8080 ENVIRONMENT=dev
cargo run -- --config config/config.toml

# Production
export BIND_ADDRESS=0.0.0.0 BIND_PORT=443 ENVIRONMENT=prod
./harmony --config /etc/harmony/config.toml
```

#### Multiple Variables

```toml
url = "$PROTOCOL://$HOST:$PORT/path"
# becomes: https://example.com:443/path (if PROTOCOL=https, HOST=example.com, PORT=443)
```

### Best Practices

1. **Use meaningful variable names**: `$API_KEY` instead of `$K`, `$BIND_ADDRESS` instead of `$BA`
2. **Document required variables**: Include a `.env.example` or README section listing all expected environment variables
3. **Use default values in scripts**: Wrap your startup in a shell script that sets defaults
   ```bash
   #!/bin/bash
   export BIND_ADDRESS=${BIND_ADDRESS:-127.0.0.1}
   export BIND_PORT=${BIND_PORT:-8080}
   exec /path/to/harmony --config config.toml
   ```
4. **Never commit secrets**: Use `.gitignore` to exclude `.env` files
5. **Log warnings**: Check logs during startup to catch missing variables early
6. **Mark critical variables as required**: Use `required_env_vars` in `[proxy]` section to enforce presence of critical variables
   ```toml
   [proxy]
   id = "harmony"
   required_env_vars = ["API_KEY", "DATABASE_PASSWORD"]
   ```
7. **Use systemd EnvironmentFile or similar**: Instead of exporting secrets in shell, use secure credential management systems
8. **Rotate secrets regularly**: Check your secret management and rotation policies
9. **Validate logs**: Ensure sensitive values don't leak into logs (only variable names are logged)

### Security Considerations

**Metadata Disclosure**
- Environment variable references are visible in your config files
- Restrict config file permissions: `chmod 600 config.toml` or `chmod 400 config.toml` (read-only after deployment)
- Configuration files should not be committed to version control if they contain `$VAR_NAME` references for sensitive values

**Missing Variables**
- By default, missing environment variables are replaced with empty strings
- Check startup logs for warnings about missing variables: `Environment variable 'X' not found`
- Use `required_env_vars` in your config to enforce critical variables and fail fast if they're missing

**Value Validation**
- Environment variable values are substituted as-is into TOML
- Ensure your values don't contain TOML metacharacters (newlines, unescaped quotes) or they may break configuration
- Startup validation (see below) can help catch malformed values

**Process Environment Visibility**
- On some systems, environment variables are visible to other users via `ps aux` or `/proc`
- For highly sensitive secrets, consider using:
  - systemd `EnvironmentFile` with restricted permissions
  - HashiCorp Vault or cloud provider secret managers (AWS Secrets Manager, Azure Key Vault)
  - Docker secrets or Kubernetes secrets for containerized deployments

**Audit Trail**
- Substituted variable names are logged at INFO level during startup
- Missing variables are logged at WARN level during startup
- Example logs:
  ```
  [INFO] Substituted 3 environment variables: API_KEY, DATABASE_PASSWORD, JWT_SECRET
  [WARN] 1 environment variable not found: MISSING_CONFIG_VAR
  ```
- Review startup logs to verify which variables were actually substituted

**Validation Command**

Test your configuration without starting the server:

```bash
./harmony --config config.toml --validate-config
# Output:
# ✓ Configuration is valid
# ✓ All required environment variables present
# ✓ Substituted 3 environment variables
# Exit code: 0
```

This validates that:
- All required variables are present
- Configuration can be parsed after substitution
- No risky values detected in environment variables

## Overview

Harmony uses a two-layer configuration model:
- **Top-level config**: Networks, storage, logging, service registrations
- **Pipeline files**: Endpoints, middleware, backends, and routing rules

**Protocol adapters** (HTTP, DIMSE, etc.) are automatically spawned based on pipeline configurations. See [adapters.md](adapters.md) for details.

Top-level config (examples/default/config.toml)
- [proxy]: service identity, paths, and JWKS cache duration
- [runbeam]: Runbeam Cloud integration settings (enabled, API URL, poll interval)
- [management]: Management API settings (enabled, base path, network)
- [network.<name>]: network interfaces and options
  - [network.<name>.http]: bind_address and bind_port
- pipelines_path: directory containing pipeline files
- transforms_path: directory for custom transforms (if used)
- [logging]: file logging options and log level
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

## Runbeam Cloud Integration

**Status**: Available since v0.4.0

### Overview

Harmony Proxy provides optional integration with Runbeam Cloud for centralized configuration management. This integration is controlled by the `[runbeam]` configuration section and is **disabled by default**.

### Configuration

```toml
[runbeam]
enabled = false  # Set to true to enable cloud integration
cloud_api_base_url = "https://api.runbeam.cloud"  # Optional, defaults to this URL
poll_interval_secs = 30  # Optional, defaults to 30 (valid range: 5-3600)
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| enabled | bool | false | Enable Runbeam Cloud integration |
| cloud_api_base_url | string | "https://api.runbeam.cloud" | Runbeam Cloud API endpoint |
| poll_interval_secs | u64 | 30 | Cloud polling interval (5-3600 seconds) |

**Important**: When `enabled = false` (default):
- No token loading from environment or storage
- No cloud polling at startup
- `/admin/authorize` endpoint returns 403 Forbidden
- Gateway runs in standalone mode without cloud connectivity

### Cloud Configuration Polling

Harmony Proxy can automatically poll Runbeam Cloud for configuration changes when running in managed mode. This enables centralized configuration management through the Runbeam Cloud dashboard.

### Overview

When authorized with a gateway token, Harmony automatically polls the Runbeam Cloud API for pending configuration changes at regular intervals (default: 30 seconds). Changes are downloaded, validated, applied to the local filesystem, and reported back to the Cloud.

### Change Processing

Changes are retrieved from the `/api/harmony/config-changes` endpoint and processed in **oldest-first order** to maintain correct configuration state progression. This ensures that configuration updates are applied in the sequence they were created.

### Change Lifecycle

1. **Queued**: Change is created on Runbeam Cloud and waiting to be fetched by the gateway
2. **Acknowledged**: Gateway has successfully fetched the change details and acknowledged receipt
3. **Applied**: Change successfully validated and applied to gateway configuration
4. **Failed**: Change application failed - error message and details reported back to Cloud

### Change Types

- **gateway**: Gateway-level configuration (main config file at `/etc/harmony/harmony-config.toml`)
- **pipeline**: Pipeline-specific configuration (individual pipeline files in `pipelines/` directory)

### How It Works

1. Gateway polls Cloud API every 30 seconds (configurable via `--poll-interval` flag)
2. API returns list of pending changes for this gateway
3. Changes processed sequentially in chronological order (oldest first)
4. For each change:
   - Fetch full change details (including TOML content)
   - Acknowledge receipt to Cloud
   - **Extract and download transforms** (if configuration references transform middleware)
   - Write TOML config to appropriate file location
   - File watcher detects change and triggers hot reload
   - Report success/failure back to Cloud

### Automatic Transform Download

When a cloud-sourced configuration references transform middleware (via `spec_path` in middleware options), Harmony automatically downloads the JOLT specifications before applying the configuration:

1. **Extract Transform IDs**: Parse the TOML configuration to find all transform middleware sections
2. **Download Specifications**: For each transform ID, call the Runbeam Cloud Transform API to retrieve the JOLT specification
3. **Write to Disk**: Save transform files to the configured `transforms_path` directory (default: `transforms/`)
4. **Overwrite Existing**: Transform files with the same ID are overwritten with the latest version
5. **Validate Before Apply**: If any transform download fails, the entire config change is marked as failed and reported to Cloud

This ensures that all transform specifications are available and up-to-date before the configuration is applied, preventing runtime errors due to missing transform files.

**Example Middleware Configuration**:
```toml
[middleware.patient_transform]
id = "01k81xgtn1hnbkfseyd82nar0m"
type = "transform"

[middleware.patient_transform.options]
spec_path = "01k81xczrw551e1qj9rgrf0319.json"  # Transform ID - automatically downloaded
apply = "both"
fail_on_error = true
```

The gateway will automatically download the JOLT specification for transform `01k81xczrw551e1qj9rgrf0319` and save it to `transforms/01k81xczrw551e1qj9rgrf0319.json` before applying the pipeline configuration.

### Configuration

Cloud polling starts automatically when the gateway is authorized:

```bash
# Start harmony with management API enabled
./harmony --config /etc/harmony/harmony-config.toml

# Authorize the gateway (via Management API)
curl -X POST http://localhost:9090/admin/authorize \
  -H "Authorization: Bearer <jwt_token>" \
  -H "Content-Type: application/json" \
  -d '{"gateway_code":"your-gateway-code"}'

# Cloud polling starts automatically after successful authorization
```

**Configuration Options**:

```toml
# Enable Runbeam Cloud integration
[runbeam]
enabled = true
cloud_api_base_url = "https://api.runbeam.cloud"
poll_interval_secs = 30  # Optional: polling interval (default: 30, range: 5-3600)

# Management API configuration
[management]
enabled = true
base_path = "/admin"
network = "default"
```

### Monitoring

Watch for cloud polling events in logs:

```
INFO 🌥️  Starting cloud config polling (interval: 30s)
INFO Processing change: id=01k8vdq9..., type=gateway, status=queued, gateway_id=01k8ek6...
INFO ✓ Successfully applied config change 01k8vdq9...
```

Errors are also logged:

```
ERROR ✗ Failed to apply config change 01k8vdq9...: Invalid TOML syntax
```

### Error Handling

**Validation Failures**:
- Invalid TOML configurations are rejected before file system operations
- Error message and details reported back to Cloud
- Previous valid configuration remains active
- Gateway continues processing remaining changes

**Network Failures**:
- Transient network errors trigger exponential backoff (2s, 4s, 8s, ...up to 5 minutes)
- Gateway continues polling when connectivity restored
- Successful poll resets error counter

**Authorization Failures**:
- 401/403 errors stop cloud polling (gateway needs re-authorization)
- Token expiry detected automatically
- Warning logged with instructions to re-authorize

### Integration with Hot Reload

Cloud-sourced configuration changes trigger the same hot-reload mechanism as file-based changes:

- **Gateway changes**: Full config reload with diff computation
- **Pipeline changes**: Selective reload of affected pipelines
- **Zero-downtime changes**: Applied immediately without service interruption
- **Adapter restarts**: Only affected networks restarted

See [Hot Configuration Reload](#hot-configuration-reload) for reload behavior details.

### Security Considerations

- **Machine tokens**: Stored securely using OS keyring (macOS Keychain, Linux Secret Service) or encrypted filesystem fallback
- **Token validation**: Tokens checked for expiry before each poll
- **HTTPS required**: All Cloud API communication uses TLS
- **Configuration integrity**: TOML validated before applying to prevent malformed configs

See [security.md](security.md) for comprehensive security guidance.

### Troubleshooting

**Polling not starting?**
- Verify gateway is authorized: check for `🌥️ Starting cloud config polling` in logs
- Check token storage: look for token file in `./tmp/runbeam/auth.json` or OS keyring
- Verify management API is enabled in config

**Changes not applying?**
- Check logs for validation errors
- Verify TOML syntax is valid
- Ensure file permissions allow writing to config directory
- Check Cloud dashboard for change status and error messages

**Token expired?**
- Re-authorize via Management API `/admin/authorize` endpoint
- Token expiry logged as warning with instructions

**Polling stopped unexpectedly?**
- Check for authorization errors (401/403) in logs
- Verify network connectivity to Cloud API
- Check for token validation failures

For more details on cloud polling architecture, see [docs/config-reload.md](config-reload.md#cloud-polling-integration).

## Environment Variables

Harmony supports several environment variables that affect runtime behavior. Most of these are **runtime settings** rather than configuration overrides - they don't replace TOML configuration but provide additional context for security, logging, and storage.

### Configuration-Affecting Variables

#### RUNBEAM_ENCRYPTION_KEY

**Purpose**: Provides encryption key for secure machine token storage when OS keyring is unavailable.

**Interaction with Configuration**:
- Does not override TOML configuration
- Affects how machine tokens are stored (see Management API authorization)
- Used automatically when OS keyring (macOS Keychain, Linux Secret Service) is unavailable
- Typical in container environments

**When to Set**:
- Production container deployments (recommended)
- Headless/CI environments
- When `RUNBEAM_DISABLE_KEYRING=1` is set (testing)

**See**: [Security Documentation](security.md#runbeam_encryption_key) for generation examples and best practices.

#### RUNBEAM_JWT_SECRET

**Purpose**: Shared secret for validating JWT tokens from Runbeam Cloud during gateway authorization.

**Interaction with Configuration**:
- Does not override TOML configuration
- Used by Management API `/authorize` endpoint
- Falls back to development default if not set (logs warning)

**When to Set**:
- Required for production Runbeam Cloud integration
- Must match secret configured in Runbeam Cloud

**See**: [Security Documentation](security.md#runbeam_jwt_secret) for generation and rotation procedures.

#### RUST_LOG

**Purpose**: Controls logging verbosity via tracing filter directives.

**Interaction with Configuration**:
- **Overrides** `logging.log_level` setting in TOML configuration
- Environment variable takes precedence when both are set
- More flexible than TOML (supports per-module filtering)

**Common Values**:
```bash
# Override log level for all Harmony modules
export RUST_LOG=harmony=debug

# Per-module filtering (overrides TOML log_level)
export RUST_LOG=harmony::router=trace,harmony::middleware=debug,harmony=info

# Global debug (very verbose)
export RUST_LOG=debug
```

**Precedence**: `RUST_LOG` environment variable > `logging.log_level` in TOML.

#### RUNBEAM_DISABLE_KEYRING

**Purpose**: Forces use of encrypted filesystem storage instead of OS keyring.

**Interaction with Configuration**:
- Does not affect TOML configuration
- Changes token storage backend selection
- Primarily for testing keyring fallback behavior

**When to Set**:
- Testing encrypted filesystem storage
- Debugging keyring-related issues
- Not needed in containers (keyring typically unavailable anyway)

### Environment Variable Precedence Rules

**Settings with Environment Variable Override**:
- **Logging**: `RUST_LOG` environment variable **overrides** `logging.log_level` in TOML

**Settings Without Override** (environment variables are supplemental):
- **Network configuration**: TOML only (bind addresses, ports, WireGuard settings)
- **Pipeline definitions**: TOML only (endpoints, backends, middleware chains)
- **Service configuration**: TOML only (service types, options)
- **Storage backend**: TOML only (filesystem path, backend type)
- **Token encryption**: `RUNBEAM_ENCRYPTION_KEY` supplements TOML (provides encryption key)
- **JWT validation**: `RUNBEAM_JWT_SECRET` supplements TOML (provides validation secret)

### Best Practices

**Development**:
```bash
# Override log level for detailed debugging
export RUST_LOG=harmony=debug,info

# Use TOML for all other configuration
cargo run -- --config examples/basic-echo/config.toml
```

**Production Containers**:
```bash
# Set security-sensitive values via environment
export RUNBEAM_ENCRYPTION_KEY=$(cat /run/secrets/encryption-key)
export RUNBEAM_JWT_SECRET=$(cat /run/secrets/jwt-secret)
export RUST_LOG=harmony=info

# Use TOML for application configuration
./harmony --config /etc/harmony/config.toml
```

**Why This Design?**
- **Security**: Secrets in environment variables (not committed to version control)
- **Configuration**: Application structure in TOML (version controlled, hot-reloadable)
- **Flexibility**: Log levels easily adjustable without config file changes
- **12-Factor App**: Environment-specific config via environment, app config via files

---

Notes
- Prefer ./tmp for temporary files rather than /tmp
- For realistic JWT auth configuration, see docs/middleware.md
- For comprehensive environment variable documentation, see [Security Documentation](security.md#environment-variables)
