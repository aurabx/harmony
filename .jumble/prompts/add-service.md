# Adding a New Service/Protocol Adapter to Harmony

## Reference Documentation
Full details: `docs/adapters.md`

## Overview

Harmony uses a unified architecture where:
- **Protocol Adapters** handle I/O (listening, parsing, response formatting)
- **PipelineExecutor** handles all business logic
- **Services** define how requests are processed

## Files to Create/Modify

1. **Adapter**: `src/adapters/<protocol>/mod.rs` - Protocol-specific I/O
2. **Service type**: `src/models/service.rs` - Add to `ServiceType` enum
3. **Protocol**: `src/models/protocol.rs` - Add to `Protocol` enum if new protocol
4. **Orchestrator**: `src/adapters/orchestrator.rs` - Register adapter startup
5. **Tests**: `tests/<protocol>/` - Integration tests

## Step-by-Step

### 1. Define the Protocol (if new)

In `src/models/protocol.rs`:
```rust
pub enum Protocol {
    Http,
    Dimse,
    YourProtocol,  // Add here
}
```

### 2. Create the Adapter

In `src/adapters/<your_protocol>/mod.rs`:

```rust
pub struct YourAdapter {
    network_name: String,
    bind_addr: SocketAddr,
}

#[async_trait]
impl ProtocolAdapter for YourAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::YourProtocol
    }

    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        // Start listener, handle connections
        // Convert protocol messages to ProtocolCtx
        // Call PipelineExecutor::execute()
        // Convert response back to protocol format
    }

    fn summary(&self) -> String {
        format!("YourAdapter on {}", self.bind_addr)
    }
}
```

### 3. Add Service Type

In `src/models/service.rs`, add to `ServiceType` enum and implement `required_protocol()`:

```rust
pub enum ServiceType {
    // ...existing...
    YourService,
}

impl ServiceType {
    pub fn required_protocol(&self) -> Protocol {
        match self {
            // ...existing...
            ServiceType::YourService => Protocol::YourProtocol,
        }
    }
}
```

### 4. Register in Orchestrator

In `src/adapters/orchestrator.rs`, add startup logic in `start_network_adapters()`.

### 5. Add Configuration Support

In `src/config/`, add configuration structs and parsing for your protocol.

Example config:
```toml
[network.default.your_protocol]
bind_address = "0.0.0.0"
bind_port = 1234
```

## Key Patterns

- **All I/O in adapter**: Never do protocol-specific I/O in services or middleware
- **Use ProtocolCtx**: Convert incoming requests to `ProtocolCtx` for unified handling
- **Use RequestEnvelope/ResponseEnvelope**: Standard types for pipeline processing
- **Graceful shutdown**: Always respect the `CancellationToken`

## Testing

Create integration tests in `tests/<protocol>/`:
```rust
#[tokio::test]
async fn test_your_protocol_basic() {
    let config = test_config_with_your_protocol();
    // Start adapter
    // Send test request
    // Verify response
}
```
