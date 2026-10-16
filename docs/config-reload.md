# Config Hot-Reload Strategy

**Last Updated**: 2026-10-16

## Overview

Harmony supports hot-reloading configuration changes without requiring a full application restart. The reload strategy uses two approaches depending on the type of change:

1. **Zero-downtime reload**: Configuration swapped atomically using `ArcSwap`
2. **Adapter restart**: Selective restart of affected protocol adapters

## Change Classification

### Zero-Downtime Changes (No Adapter Restart)

These changes are picked up immediately on the next request with no service interruption. HTTP, HTTP/3 and DIMSE requests all read the live config. For HTTP and HTTP/3, route lookup and the pipeline run use one snapshot, read when the request arrives.

- **Middleware configuration**
  - Transform specs
  - Auth rules and policies, including the top-level `[policies]` and `[rules]` tables
  - Custom middleware options
  
- **Route definitions**
  - Groups, endpoints, backends
  - Path patterns and matching rules. Each HTTP router rebuilds its route table the first time a request sees a new config.
  - Request/response transforms
  
- **Backend configuration**
  - Target URLs
  - Timeouts and retry policies
  - Connection pool settings
  
- **Other tables read when used**
  - `[authentications]`, `[targets]`, `[peers]` and `[management]`
  - Targets, peers and management are merged into backends, endpoints and pipelines at load, so their effect also shows there

### Adapter Restart Required

These changes require restarting specific protocol adapters (brief interruption for affected networks only):

- **Network topology**
  - Bind addresses
  - Bind ports
  - Adding/removing networks

- **Adapter set**
  - A network starts only the adapters its pipelines need: HTTP and HTTP/3 for HTTP endpoints, DIMSE for `dicom_scp` endpoints
  - Adding the first or removing the last pipeline of a protocol on a network restarts that network

- **Mesh routing**
  - Any change to `[mesh]`, `[ingress]`, `[egress]` or `[remote_ingress]` restarts every network
  - Each HTTP router indexes mesh ingress URLs when it starts
  - Networks restart one at a time. Until a network restarts, a request that matches one of its mesh ingress URLs gets 503, so it never runs under mixed old and new mesh rules
  
- **Protocol-specific settings**
  - TLS certificates
  - Connection pool sizes
  - Protocol handler options

- **WireGuard configuration**
  - Interface names
  - Peer settings

- **DIMSE listeners**
  - Any change to the `dicom_scp` endpoints or persistent-SCP backends a network serves, or to their options (port, bind address, AE title, enabled operations, `use_dcmtk_store`, `persistent_store_scp`)
  - Adding the first or removing the last `dicom_scp` endpoint on a network
  - This includes every option of a legacy `dicom_scu` backend with `persistent_store_scp` on a network that has a `dicom_scp` endpoint, even `host` or `aet`. Such a change restarts every adapter on that network.
  - The listener reads these options only when it starts. Within a network, the restart stops the old listener, including a DCMTK `storescp` process, before the new one binds the port. Networks restart in no fixed order, so a port moving from one network to another may fail to bind until the other network has restarted.

### Process Restart Required

These tables are read wholly or partly at startup:

- `[logging]`: the log subscriber and log file are set up once. `RUST_LOG` is also read only at startup.
- `[storage]`: the storage backend is created once.
- `[provider]`: the provider resolver is updated on reload, but cloud polling keeps its old interval.
- `[proxy]`: file paths are read at startup, and some fields are read live.
- `[runbeam]`: cloud polling starts at startup, and some fields are read live.
- `[services]` and `[middleware_types]`: these fill registries that can be set only once.
- `[extensions]`: plugins read these when they start.

A reload that changes them still swaps in the new config, so anything that reads the live config sees the new values. The watcher logs a warning naming the tables. Restart the process for full effect.

`compute_diff` lists every `Config` field by name. Adding a field fails to compile until it is put in one of these groups.

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
// Global config storage (src/globals.rs)
static CONFIG: Lazy<ArcSwapOption<Config>> = ...;

// Readers (no locks, no copy): hands out the published Arc
let config: Option<Arc<Config>> = globals::get_config();

// Writers (atomic swap)
globals::set_config(Arc::new(new_config));
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

- The new config is swapped in before any adapter is stopped, restarted or started. Restarted adapters and the DIMSE peer check never run under the old config.
- Every network in the diff is attempted, even after one fails.
- The reload returns an error naming each network that failed. The new config stays active, because it was validated before this point.
- A network whose restart failed may be left stopped. The watcher remembers it and restarts it on the next reload, even if that reload changes nothing. To retry after fixing an outside cause, such as a port in use, save the config file again.

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
