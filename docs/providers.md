# Providers

Providers define how Harmony resolves and synchronizes resources. A provider can be local (configuration files on disk) or remote (a cloud service like Runbeam Cloud).

## Overview

Providers serve two main purposes:

1. **Resource Resolution**: Resolve references to resources like ingresses, egresses, and meshes
2. **Configuration Sync**: Poll remote APIs for configuration changes (remote providers only)

Every Harmony gateway has an implicit `local` provider that resolves resources from local configuration files. You can configure additional providers to connect to remote services.

## Primary Provider

The `primary_provider` setting in the `[proxy]` section determines which provider is used for cloud polling:

```toml
[proxy]
id = "my-gateway"
primary_provider = "runbeam"  # Default: Use Runbeam Cloud for polling
```

### Options

| Value | Description |
|-------|-------------|
| `runbeam` | Use Runbeam Cloud for configuration sync (default) |
| `local` | Disable cloud polling, use only local configuration |
| `<custom>` | Use a custom provider defined in `[provider.*]` |

### Example: Local-Only Gateway

```toml
[proxy]
id = "standalone-gateway"
primary_provider = "local"
```

This gateway will not poll any remote service for configuration changes and uses only local configuration files.

## Provider Configuration

Configure providers using `[provider.*]` sections:

```toml
[provider.runbeam]
api = "https://api.runbeam.cloud"
poll_interval_secs = 30

[provider.custom-provider]
api = "https://custom-api.example.com"
poll_interval_secs = 60
```

### Provider Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `api` | string | Yes* | - | Base URL for the provider API |
| `poll_interval_secs` | integer | No | 30 | Polling interval in seconds (0 = disabled) |

*Required for remote providers. The `local` provider doesn't need an `api` field.

### Polling Intervals

The `poll_interval_secs` controls how frequently Harmony checks the provider for configuration changes:

- **Minimum**: 0 (disables polling)
- **Maximum**: 3600 (1 hour)
- **Recommended**: 30 seconds for active development, 60-300 for production

```toml
[provider.runbeam]
api = "https://api.runbeam.cloud"
poll_interval_secs = 60  # Check every minute
```

Set to `0` to disable polling while keeping the provider available for reference resolution:

```toml
[provider.runbeam]
api = "https://api.runbeam.cloud"
poll_interval_secs = 0  # No polling, but can still resolve references
```

## Built-in Providers

### Local Provider

The `local` provider is always available implicitly. It resolves resources from your local configuration files.

```toml
[proxy]
primary_provider = "local"
# No [provider.local] section needed
```

**Characteristics**:
- No API URL required
- No polling (poll_interval_secs = 0)
- Resolves resources from `pipelines/`, `mesh/`, and other config directories

**Use Cases**:
- Standalone deployments
- Development environments
- On-premises gateways without cloud connectivity

### Runbeam Provider

The Runbeam provider connects to Runbeam Cloud for centralized configuration management:

```toml
[provider.runbeam]
api = "https://api.runbeam.cloud"
poll_interval_secs = 30
```

**Characteristics**:
- Resolves resources from Runbeam Cloud API
- Polls for configuration changes when authorized
- Supports team-scoped resources

**When to Use**:
- Multi-team or multi-organization deployments
- Centralized configuration management
- Cross-gateway resource sharing

**Setup**:
1. Configure provider in `config.toml`
2. Start Harmony with management API enabled
3. Authorize the gateway via Management API `/admin/authorize` endpoint
4. Polling starts automatically after authorization

Example configuration:

```toml
[proxy]
id = "my-gateway"
primary_provider = "runbeam"

[provider.runbeam]
api = "https://api.runbeam.cloud"
poll_interval_secs = 30

[management]
enabled = true
network = "default"
```

Authorize the gateway:

```bash
curl -X POST http://localhost:9090/admin/authorize \
  -H "Authorization: Bearer <jwt_token>" \
  -H "Content-Type: application/json" \
  -d '{"gateway_code": "your-gateway-code"}'
```

## Configuration Polling

When configured as the primary provider, a remote provider automatically polls for configuration changes at regular intervals.

### How It Works

1. Gateway polls provider API every `poll_interval_secs` seconds
2. API returns list of pending changes for this gateway
3. Changes are processed sequentially in chronological order (oldest first)
4. For each change:
   - Fetch full change details (including TOML content)
   - Acknowledge receipt to provider
   - Validate and apply to local files
   - Report success/failure back to provider

### Change Processing

Changes are applied using the same hot-reload mechanism as file-based changes:

- **Zero-downtime changes**: Middleware, routes, backends updated immediately
- **Adapter restarts**: Network changes require brief interruption (~1-2s)

See [Hot Configuration Reload](./config-reload.md) for detailed reload behavior.

### Automatic Transform Download

When cloud-sourced configuration references transform middleware, Harmony automatically downloads the JOLT specifications before applying the configuration.

This ensures all transform files are available and up-to-date before configuration is applied, preventing runtime errors due to missing transform files.

## Migration from Legacy Configuration

If you're using the legacy `[runbeam]` section, migrate to the provider-based configuration:

### Before (Legacy)

```toml
[runbeam]
enabled = true
cloud_api_base_url = "https://api.runbeam.cloud"
poll_interval_secs = 30
```

### After (Provider-based)

```toml
[proxy]
id = "my-gateway"
primary_provider = "runbeam"

[provider.runbeam]
api = "https://api.runbeam.cloud"
poll_interval_secs = 30
```

**Notes**:
- The legacy `[runbeam]` section is still supported for backward compatibility but is deprecated
- New deployments should use the provider-based configuration
- Both configurations cannot coexist; choose one or the other

### Why Migrate?

The provider-based configuration:
- Supports multiple remote providers simultaneously (future enhancement)
- Explicitly separates primary provider choice from configuration
- Aligns with resource reference syntax
- More flexible and extensible

## Current Limitations

Currently, Harmony supports only a single active remote provider at a time:

- **Primary Provider Setting**: Determines which provider is active for polling and reference resolution
- **Multiple Provider Definitions**: You can define multiple `[provider.*]` sections, but only the primary provider is used for polling
- **Reference Resolution**: Uses the primary provider for resolving non-local references

You can switch between providers by:
1. Changing `primary_provider` setting
2. Restarting Harmony (or hot-reloading config)

Example with multiple providers defined (only one is active):

```toml
[proxy]
primary_provider = "runbeam"  # This one is active

[provider.runbeam]
api = "https://api.runbeam.cloud"
poll_interval_secs = 30

[provider.backup-provider]
api = "https://backup-api.example.com"
poll_interval_secs = 0  # Defined but not used
```

## Reference Resolution

Providers are used to resolve resource references in mesh and other configurations:

```toml
# Local reference (always works, no provider needed)
ingress = ["local.name.my-ingress"]

# Provider reference (uses configured provider)
ingress = ["runbeam.partner-team.ingress.name.their-api"]
```

See [Resource References](./resource-references.md) for complete documentation on reference syntax and resolution.

## Monitoring

Watch provider polling events in logs:

```
INFO 🌥️  Starting cloud config polling (interval: 30s)
INFO Processing change: id=01k8vdq9..., type=gateway, status=queued
INFO ✓ Successfully applied config change 01k8vdq9...
```

Errors are also logged:

```
ERROR ✗ Failed to apply config change 01k8vdq9...: Invalid TOML syntax
ERROR Network error connecting to provider: Connection refused
WARN Token expired, re-authorization needed
```

## Error Handling

### Validation Failures

- Invalid TOML configurations are rejected before file system operations
- Error message and details reported back to provider
- Previous valid configuration remains active
- Gateway continues processing remaining changes

### Network Failures

- Transient network errors trigger exponential backoff (2s, 4s, 8s, ... up to 5 minutes)
- Gateway continues polling when connectivity restored
- Successful poll resets error counter

### Authorization Failures

- 401/403 errors stop cloud polling (gateway needs re-authorization)
- Token expiry detected automatically
- Warning logged with instructions to re-authorize

## Security Considerations

- **Machine Tokens**: Stored securely using encrypted filesystem
- **Token Validation**: Tokens checked for expiry before each poll
- **HTTPS Required**: All cloud API communication uses TLS
- **Configuration Integrity**: TOML validated before applying to prevent malformed configs

See [Security Documentation](./security.md) for comprehensive security guidance.

## Troubleshooting

**Polling not starting?**
- Verify gateway is authorized: check for `🌥️ Starting cloud config polling` in logs
- Check token storage
- Verify management API is enabled in config

**Changes not applying?**
- Check logs for validation errors
- Verify TOML syntax is valid
- Ensure file permissions allow writing to config directory

**Token expired?**
- Re-authorize via Management API `/admin/authorize` endpoint
- Token expiry is logged as warning with instructions

**Polling stopped unexpectedly?**
- Check for authorization errors (401/403) in logs
- Verify network connectivity to provider API
- Check for token validation failures

## See Also

- [Configuration Schemas](./schema.md) - Provider schema reference
- [Resource References](./resource-references.md) - Reference syntax and resolution
- [Data Mesh](./mesh.md) - Using providers in mesh configurations
- [Configuration](./configuration.md) - Main configuration documentation
