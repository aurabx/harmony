---
title: Runbeam Cloud Integration
description: How to connect Harmony to Runbeam Cloud for authorisation and configuration polling.
status: active
updated: 2026-10-24
type: Integration
related:
  - configuration/configuration-file.md
---

# Runbeam Cloud Integration

How to connect Harmony to Runbeam Cloud for authorisation and configuration polling. This page is part of [Configuration](../configuration/configuration-file.md).

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
| poll_interval_secs | u64 | 30 | Cloud polling interval (5-3600 seconds). Used only when `enabled = true`, the primary provider is `runbeam`, and no `[provider.runbeam]` table exists. A `[provider.runbeam]` table overrides it. |

**Important**: `enabled` does not control everything on its own.

- Harmony loads tokens and starts cloud polling when `enabled = true`. It also does so when the primary provider (`proxy.primary_provider`, default `runbeam`) is a `[provider.<name>]` table with an `api` URL and a non-zero `poll_interval_secs`.
- When neither is true, Harmony loads no token, does not poll, and runs in standalone mode.
- The `/admin/authorize` endpoint checks only `enabled`. With `enabled = false` it returns 403 Forbidden, even when a provider table turns polling on.

### Cloud Configuration Polling

Harmony Proxy can automatically poll Runbeam Cloud for configuration changes when running in managed mode. This enables centralized configuration management through the Runbeam Cloud dashboard.

### Overview

When authorized with a gateway token, Harmony automatically polls the Runbeam Cloud API for pending configuration changes at regular intervals (default: 30 seconds). Harmony downloads each change, writes it to the local filesystem, and reports it back to the Cloud. The file watcher then validates and applies it. See [Integration with Hot Reload](#integration-with-hot-reload).

### Change Processing

Changes are retrieved from the `/api/harmony/config-changes` endpoint and processed in **oldest-first order** to maintain correct configuration state progression. This ensures that configuration updates are applied in the sequence they were created.

### Change Lifecycle

1. **Queued**: Change is created on Runbeam Cloud and waiting to be fetched by the gateway
2. **Acknowledged**: Gateway has successfully fetched the change details and acknowledged receipt
3. **Applied**: Harmony wrote the change to disk. Harmony reports this before the reload runs, so it does not mean the reload succeeded.
4. **Failed**: Harmony could not download the change's transforms or write the file. Harmony reports the error to Cloud. Reload failures are not reported.

### Change Types

- **gateway**: Gateway-level configuration. Written to the active config file, the path passed with `--config`. An unknown type is also written there.
- **pipeline**: Pipeline configuration. Written to `<pipelines_path>/<name>.toml`.
- **transform**: A JOLT specification. Written to `<transforms_path>/<name>.json`.
- **mesh**: Mesh configuration. Written to `<mesh_path>/<name>.toml`.

These paths are relative to the config file's directory.

### How It Works

1. Gateway polls Cloud API every 30 seconds by default. The interval comes from the primary provider's `poll_interval_secs`, read at startup. Without a `[provider.runbeam]` table, that value comes from `[runbeam].poll_interval_secs`, but only when `[runbeam].enabled = true`. See [Providers](../configuration/providers.md).
2. API returns list of pending changes for this gateway
3. Changes processed sequentially in chronological order (oldest first)
4. For each change:
   - Fetch full change details (including TOML content)
   - Acknowledge receipt to Cloud
   - **Extract and download transforms** (if configuration references transform middleware)
   - Write TOML config to the right file location, and a backup copy to `./tmp/cloud_configs/`
   - Report the change as applied to Cloud
   - The file watcher detects the change and runs a hot reload

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
  -d '{"gateway_id":"your-gateway-id"}'

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
base_path = "admin"  # no leading slash; "/admin" produces routes like //admin/info
network = "default"
```

### Monitoring

Watch for cloud polling events in logs:

```
INFO 🌥️  Starting cloud config polling (interval: 30s)
INFO Processing change: id=01k8vdq9..., type=gateway, status=queued, gateway_id=01k8ek6...
INFO ✓ Wrote config change 01k8vdq9... to /etc/harmony/harmony-config.toml (file watcher will apply)
```

The reload result appears in the file watcher's log lines. See [Hot Configuration Reload](../configuration/hot-reload.md#errors).

Write errors are also logged:

```
ERROR ✗ Failed to write config change 01k8vdq9...: Failed to write config file: <reason>
```

### Error Handling

**Validation Failures**:
- The poller does not validate a change before it writes the file
- The file watcher validates it on reload. If it is invalid, the previous valid configuration remains active
- The validation error appears in the gateway log only. Cloud still shows the change as applied
- Gateway continues processing remaining changes

**Network Failures**:
- After the first error, Harmony waits only the normal poll interval. From the second error in a row, it also backs off for 2^n seconds, where n is the error count: 4s, 8s, 16s, and so on. The backoff runs before the normal poll interval wait. The exponent stops at 8, so the longest backoff is 256s
- Gateway continues polling when connectivity restored
- Successful poll resets error counter

**Authorization Failures**:
- 401/403 errors stop cloud polling (gateway needs re-authorization)
- Token expiry detected automatically
- Warning logged with instructions to re-authorize

### Integration with Hot Reload

The poller does not reload the config itself. It writes files, and the file watcher reloads them, the same as a manual edit:

- **Gateway changes**: written to the main config file. The watcher reloads it.
- **Pipeline changes**: written to `pipelines/<name>.toml`. The watcher runs a full reload, but only if the pipelines directory existed when Harmony started. Otherwise the change applies on the next reload of the main config file. There is no per-pipeline reload.
- **Transform and mesh changes**: written to the transforms and mesh directories. The watcher does not watch these, so they apply on the next reload of a watched file.

See [Hot Configuration Reload](../configuration/hot-reload.md) for reload behavior details.

### Security Considerations

- **Machine tokens**: Stored securely using encrypted filesystem (age X25519 encryption)
- **Token validation**: Tokens checked for expiry before each poll
- **HTTPS required**: All Cloud API communication uses TLS
- **Configuration integrity**: TOML validated before applying to prevent malformed configs

See [Security](../security/overview.md) for comprehensive security guidance.

### Troubleshooting

**Polling not starting?**
- Verify gateway is authorized: check for `🌥️ Starting cloud config polling` in logs
- Check token storage: look for the encrypted token file at `~/.runbeam/<proxy_id>/runbeam/auth.json`
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

For more details on cloud polling architecture, see [docs/architecture/config-reload.md](../architecture/config-reload.md#cloud-poller-handoff).
