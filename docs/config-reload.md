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
