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

See [Hot Configuration Reload](../configuration/hot-reload.md#hot-configuration-reload-1) for reload behavior details.

### Security Considerations

- **Machine tokens**: Stored securely using encrypted filesystem (age X25519 encryption)
- **Token validation**: Tokens checked for expiry before each poll
- **HTTPS required**: All Cloud API communication uses TLS
- **Configuration integrity**: TOML validated before applying to prevent malformed configs

See [security.md](../security/overview.md) for comprehensive security guidance.

### Troubleshooting

**Polling not starting?**
- Verify gateway is authorized: check for `🌥️ Starting cloud config polling` in logs
- Check token storage: look for token file in `~/.runbeam/<proxy_id>/auth.json`
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

For more details on cloud polling architecture, see [docs/architecture/config-reload.md](../architecture/config-reload.md#cloud-polling-automatic).
