# Config Hot-Reload Strategy

## Overview

Harmony supports hot-reloading configuration changes without requiring a full application restart. The reload strategy uses two approaches depending on the type of change:

1. **Zero-downtime reload**: Configuration swapped atomically using `ArcSwap`
2. **Adapter restart**: Selective restart of affected protocol adapters

## Change Classification

### Zero-Downtime Changes (No Adapter Restart)

These changes are picked up immediately on the next request with no service interruption:

- **Middleware configuration**
  - Transform specs
  - Auth rules and policies
  - Custom middleware options
  
- **Route definitions**
  - Groups, endpoints, backends
  - Path patterns and matching rules
  - Request/response transforms
  
- **Backend configuration**
  - Target URLs
  - Timeouts and retry policies
  - Connection pool settings
  
- **Logging settings**
  - Log levels
  - File paths
  - Output formats

- **Storage configuration**
  - Backend type (filesystem, database)
  - Paths and connection strings

### Adapter Restart Required

These changes require restarting specific protocol adapters (brief interruption for affected networks only):

- **Network topology**
  - Bind addresses
  - Bind ports
  - Adding/removing networks
  
- **Protocol-specific settings**
  - TLS certificates
  - Connection pool sizes
  - Protocol handler options

- **WireGuard configuration**
  - Interface names
  - Peer settings

## Implementation Architecture

### Components

```
┌─────────────────────┐
│   File Watcher      │ (notify crate, debounced)
│   (config.toml)     │
└──────────┬──────────┘
           │ detects change
           ▼
┌─────────────────────┐
│ Config Validator    │
│ + Diff Calculator   │
└──────────┬──────────┘
           │
           ├─ validation fails ──► Log error, keep old config
           │
           ├─ zero-downtime ─────► ArcSwap::store(new_config)
           │
           └─ adapter restart ───► Cancel specific adapters
                                   ▼
                              Spawn new adapters
```

### ArcSwap Pattern

```rust
// Global config storage
static CONFIG: ArcSwap<Config> = ArcSwap::from_pointee(...);

// Readers (no locks, zero-copy)
let config = CONFIG.load();
let bind_addr = config.network["default"].http.bind_address;

// Writers (atomic swap)
CONFIG.store(Arc::new(new_config));
```

### Adapter Registry

```rust
struct AdapterRegistry {
    adapters: HashMap<String, AdapterHandle>,  // network_name -> handle
}

// Selective restart
registry.restart_network("default", new_config).await;
```

## Reload Triggers

### Automatic (File Watcher)

- Watches `config.toml` for modifications
- 200ms debounce to handle editor save patterns
- Validates → Diffs → Applies changes automatically

### Manual (Management API)

```bash
# Trigger reload
curl -X POST http://localhost:9090/api/reload

# Check config status
curl http://localhost:9090/api/config/status
```

Response:
```json
{
  "status": "reloaded",
  "diff": {
    "zero_downtime_changes": ["middleware.transform.spec_path"],
    "adapter_restarts_required": []
  },
  "restarted_networks": [],
  "last_reload": "2025-10-31T01:15:00Z"
}
```

### Cloud Polling (Automatic)

When running in managed mode with cloud polling enabled, configuration changes from Runbeam Cloud automatically trigger hot reloads.

#### How It Works

1. **Poller fetches pending changes** from Cloud API (every 30 seconds by default)
2. **Changes processed in chronological order** (oldest first) to maintain state consistency
3. **Each change written to filesystem**:
   - Gateway changes → main `harmony-config.toml` (or configured path)
   - Pipeline changes → `pipelines/{pipeline_id}.toml`
4. **File watcher detects change** and triggers hot reload mechanism
5. **Hot reload validates and applies** the new configuration (zero-downtime or adapter restart)
6. **Status reported back to Cloud** (applied/failed with error details)

#### Change Application Flow

```
┌──────────────────┐
│  Runbeam Cloud   │
│   API Endpoint   │
└────────┬─────────┘
         │ poll (30s)
         ▼
┌──────────────────┐
│  Cloud Poller    │
│  (background)    │
└────────┬─────────┘
         │ fetch changes
         ▼
┌──────────────────┐
│  Acknowledge     │ ◄─── Change ID, status="acknowledged"
│  to Cloud        │
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│  Write TOML to   │ ◄─── Gateway: ./tmp/cloud_config_{id}.toml
│  Filesystem      │      Pipeline: pipelines/{pipeline_id}.toml
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│  File Watcher    │ ◄─── Detects file modification
│  Triggers        │
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│  Hot Reload      │ ◄─── Zero-downtime or adapter restart
│  Mechanism       │
└────────┬─────────┘
         │
         ├─ success ────► Report applied_at to Cloud
         │
         └─ failure ────► Report failed_at + error details to Cloud
```

#### Change Types

- **Gateway changes**: Full config reload with diff computation
  - Affects top-level config (networks, storage, logging)
  - Written to main config file path
  - May trigger adapter restarts if network topology changes
  
- **Pipeline changes**: Selective reload of affected pipeline
  - Affects endpoints, middleware, backends for specific pipeline
  - Written to `pipelines/{pipeline_id}.toml`
  - Typically zero-downtime unless network bindings change

#### Monitoring

Watch for cloud polling events in logs:

```
INFO 🌥️  Starting cloud config polling (interval: 30s)
INFO Processing change: id=01k8vdq9..., type=gateway, status=queued, gateway_id=01k8ek6..., created_at=2025-10-30T20:42:36Z
INFO Wrote cloud config to ./tmp/cloud_config_01k8vdq9....toml
INFO ✓ Successfully applied config change 01k8vdq9...
```

For failed changes:

```
ERROR ✗ Failed to apply config change 01k8vdq9...: Configuration validation failed
```

#### Error Recovery

If a change fails to apply:
- **Error logged** with full details (validation errors, file I/O errors, etc.)
- **Error reported to Cloud** with `error_message` and `error_details` fields
- **Previous valid configuration remains active** - no service disruption
- **Proxy continues processing remaining changes** in the queue
- **Failed change can be retried** from Cloud dashboard after fixing

#### Configuration

Cloud polling is enabled automatically when the gateway is authorized:

```toml
[management]
enabled = true
base_path = "/admin"
network = "default"
poll_interval_secs = 30  # Optional: override default polling interval
```

See [configuration.md](configuration.md#cloud-configuration-polling) for authorization and setup details.

## Error Handling

### Invalid Config

- Validation errors logged with details
- Old configuration retained
- Application continues running
- API returns validation errors

### Partial Reload Failure

- If adapter restart fails, old adapter keeps running
- Error logged with network name
- Other networks unaffected

## Testing Strategy

### Zero-Downtime Reload Test

```rust
#[tokio::test]
async fn test_middleware_config_reload() {
    // 1. Start with initial config
    // 2. Send concurrent requests
    // 3. Swap config (change transform spec)
    // 4. Verify all requests succeed
    // 5. Verify new requests use new config
}
```

### Adapter Restart Test

```rust
#[tokio::test]
async fn test_network_change_reload() {
    // 1. Start with network on port 8080
    // 2. Change port to 8081
    // 3. Verify old adapter shuts down
    // 4. Verify new adapter starts on 8081
    // 5. Verify other networks unaffected
}
```

### Invalid Config Test

```rust
#[tokio::test]
async fn test_invalid_config_rejected() {
    // 1. Start with valid config
    // 2. Attempt reload with invalid config
    // 3. Verify old config still active
    // 4. Verify application continues running
}
```

## Limitations

- **In-flight requests**: Requests in progress during adapter restart may fail
- **Connection draining**: No graceful connection draining (future enhancement)
- **Config rollback**: No automatic rollback on partial failure (future enhancement)
- **Audit trail**: No config change history tracking (future enhancement)

## Best Practices

1. **Test config changes**: Validate config locally before deploying
2. **Use management API**: For controlled deployments, trigger reload via API
3. **Monitor logs**: Watch for reload events and validation errors
4. **Staged rollout**: Change non-critical settings first, then network topology
5. **Backup configs**: Keep previous config versions for quick rollback

## Future Enhancements

- Connection draining during adapter restart
- Config change audit log
- Automatic rollback on validation failure
- Config versioning and diff history
- Reload dry-run mode
