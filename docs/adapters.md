# Protocol Adapters

**Last Updated**: 2025-01-18 (Phase 6)

## Overview

Protocol adapters are the foundation of Harmony's protocol-agnostic architecture. Each protocol (HTTP, DIMSE, HL7, etc.) has a dedicated adapter that handles protocol-specific I/O while using the unified `PipelineExecutor` for all business logic.

## Architecture

```
Protocol Request
  ↓
ProtocolAdapter
  ├─ Protocol-specific I/O (listening, parsing)
  ├─ Convert to ProtocolCtx + RequestEnvelope
  ├─ Call PipelineExecutor (unified logic)
  ├─ Convert ResponseEnvelope back
  └─ Protocol-specific response formatting
  ↓
Protocol Response
```

**Key Principle**: Protocol adapters handle I/O; `PipelineExecutor` handles business logic.

## ProtocolAdapter Trait

All adapters implement the `ProtocolAdapter` trait:

```rust
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    /// Returns the protocol this adapter handles
    fn protocol(&self) -> Protocol;

    /// Start the adapter (listener, server, etc.)
    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>>;

    /// Returns a human-readable summary
    fn summary(&self) -> String;
}
```

## Available Adapters

### HttpAdapter

**Location**: `src/adapters/http/`

**Protocol**: HTTP/HTTPS

**Features**:
- Axum-based web server
- Route matching and conflict detection
- Header and body mapping to/from envelopes
- Support for all HTTP methods (GET, POST, PUT, DELETE, etc.)

**Usage**:
```rust
let adapter = HttpAdapter::new(network_name, bind_addr);
let handle = adapter.start(config, shutdown).await?;
```

### DimseAdapter

**Location**: `src/adapters/dimse/`

**Protocol**: DICOM DIMSE

**Features**:
- DIMSE SCP (Service Class Provider) listener
- Support for C-FIND, C-STORE, C-MOVE, C-ECHO
- AE title-based routing
- Dataset encoding/decoding

**Usage**:
```rust
let adapter = DimseAdapter::new(network_name);
let handle = adapter.start(config, shutdown).await?;
```

## Implementing a New Adapter

### Example: HL7 MLLP Adapter

Here's how to implement an adapter for HL7 over MLLP (Minimal Lower Layer Protocol):

#### 1. Create the Adapter Structure

```rust
// src/adapters/hl7_mllp/mod.rs

use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::models::protocol::{Protocol, ProtocolCtx};
use crate::pipeline::PipelineExecutor;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub struct Hl7MllpAdapter {
    network_name: String,
    bind_addr: SocketAddr,
}

impl Hl7MllpAdapter {
    pub fn new(network_name: String, bind_addr: SocketAddr) -> Self {
        Self {
            network_name,
            bind_addr,
        }
    }
}
```

#### 2. Implement Protocol-Specific Conversion

```rust
impl Hl7MllpAdapter {
    /// Convert HL7 message to ProtocolCtx
    fn hl7_to_protocol_ctx(&self, message: &[u8]) -> ProtocolCtx {
        // Parse HL7 message segments
        let message_str = String::from_utf8_lossy(message);
        let mut meta = HashMap::new();
        
        // Extract MSH segment (message header)
        if let Some(msh) = message_str.lines().next() {
            let fields: Vec<&str> = msh.split('|').collect();
            if fields.len() > 8 {
                meta.insert("message_type".to_string(), fields[8].to_string());
            }
        }
        
        ProtocolCtx {
            protocol: Protocol::Hl7,
            payload: message.to_vec(),
            meta,
            attrs: serde_json::json!({
                "encoding": "ER7", // HL7 v2 pipe-delimited format
            }),
        }
    }

    /// Convert ResponseEnvelope to HL7 ACK
    fn envelope_to_hl7_ack(&self, envelope: ResponseEnvelope<Vec<u8>>) -> Vec<u8> {
        // Generate HL7 ACK message
        let ack_code = if envelope.response_details.status == 200 {
            "AA" // Application Accept
        } else {
            "AE" // Application Error
        };
        
        format!(
            "MSH|^~\\&|HARMONY|||{}||ACK|{}|P|2.5\rMSA|{}|{}\r",
            chrono::Utc::now().format("%Y%m%d%H%M%S"),
            uuid::Uuid::new_v4(),
            ack_code,
            // Include message control ID from original message
            envelope.request_details.metadata.get("message_control_id")
                .unwrap_or(&"".to_string())
        ).into_bytes()
    }
}
```

#### 3. Implement ProtocolAdapter Trait

```rust
#[async_trait]
impl ProtocolAdapter for Hl7MllpAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::Hl7
    }

    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let bind_addr = self.bind_addr;
        let network_name = self.network_name.clone();

        Ok(tokio::spawn(async move {
            let listener = TcpListener::bind(bind_addr)
                .await
                .expect("Failed to bind HL7 MLLP listener");

            tracing::info!("HL7 MLLP listener started on {}", bind_addr);

            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (socket, addr) = result.expect("Accept failed");
                        let config = config.clone();
                        
                        tokio::spawn(async move {
                            if let Err(e) = handle_hl7_connection(
                                socket, 
                                addr, 
                                config
                            ).await {
                                tracing::error!("HL7 connection error: {}", e);
                            }
                        });
                    }
                    _ = shutdown.cancelled() => {
                        tracing::info!("HL7 MLLP adapter shutting down");
                        break;
                    }
                }
            }
        }))
    }

    fn summary(&self) -> String {
        format!("Hl7MllpAdapter on {}", self.bind_addr)
    }
}

async fn handle_hl7_connection(
    socket: TcpStream,
    addr: SocketAddr,
    config: Arc<Config>,
) -> anyhow::Result<()> {
    // Read MLLP-framed message (0x0B start, 0x1C 0x0D end)
    let message = read_mllp_message(&socket).await?;
    
    // Convert to ProtocolCtx
    let adapter = Hl7MllpAdapter::new("hl7_network".into(), addr);
    let ctx = adapter.hl7_to_protocol_ctx(&message);
    
    // Get pipeline configuration
    let pipeline = config.pipelines.get("hl7_pipeline")
        .ok_or_else(|| anyhow::anyhow!("HL7 pipeline not found"))?;
    
    // Get endpoint and build envelope
    let endpoint = config.endpoints.get("hl7_endpoint")
        .ok_or_else(|| anyhow::anyhow!("HL7 endpoint not found"))?;
    let service = endpoint.resolve_service()?;
    let envelope = service.build_protocol_envelope(
        ctx.clone(),
        endpoint.options.as_ref().unwrap_or(&HashMap::new())
    ).await?;
    
    // Execute through unified pipeline
    let response = PipelineExecutor::execute(envelope, pipeline, &config, &ctx)
        .await?;
    
    // Convert back to HL7 ACK and send
    let ack = adapter.envelope_to_hl7_ack(response);
    write_mllp_message(&socket, &ack).await?;
    
    Ok(())
}
```

#### 4. Register in Orchestrator

```rust
// In src/lib.rs::run()

// Start HL7 MLLP adapter if network has HL7 endpoints
let has_hl7 = config.pipelines.values().any(|pipeline| {
    pipeline.networks.contains(&network_name)
        && pipeline.endpoints.iter().any(|endpoint_name| {
            config.endpoints.get(endpoint_name)
                .map(|e| e.service == "hl7")
                .unwrap_or(false)
        })
});

if has_hl7 {
    let hl7_adapter = Hl7MllpAdapter::new(network_name.clone(), hl7_bind_addr);
    match hl7_adapter.start(config_clone, shutdown_clone).await {
        Ok(handle) => {
            tracing::info!("🚀 Started HL7 MLLP adapter for network '{}'", network_name);
            adapter_handles.push(handle);
        }
        Err(e) => {
            tracing::error!("Failed to start HL7 MLLP adapter: {}", e);
        }
    }
}
```

## Best Practices

### 1. Separation of Concerns
- **Adapter**: Protocol I/O only (parsing, formatting, framing)
- **PipelineExecutor**: Business logic (auth, transforms, backends)
- **Service**: Protocol-agnostic data mapping

### 2. Error Handling
- Convert protocol errors to `PipelineError`
- Map pipeline errors to protocol-specific status codes
- Log errors with appropriate protocol context

### 3. Graceful Shutdown
- Always respect the `CancellationToken`
- Clean up resources (close sockets, release file handles)
- Wait for in-flight requests to complete if possible

### 4. Observability
- Add tracing spans with protocol-specific metadata
- Include request IDs for correlation across adapters
- Log key protocol events (connections, errors, status changes)

### 5. Testing
- Unit test protocol conversions (to/from ProtocolCtx)
- Integration test with PipelineExecutor
- Test graceful shutdown and error handling
- Use hermetic test data from `/samples`

## Common Patterns

### Request ID Propagation
```rust
let request_id = uuid::Uuid::new_v4().to_string();
meta.insert("request_id".to_string(), request_id.clone());

tracing::info!(
    request_id = %request_id,
    protocol = ?Protocol::Hl7,
    "Processing request"
);
```

### Protocol Metadata
```rust
// Store protocol-specific metadata in ProtocolCtx.attrs
let ctx = ProtocolCtx {
    protocol: Protocol::Custom,
    payload: data,
    meta: HashMap::from([
        ("protocol".to_string(), "custom".to_string()),
        ("version".to_string(), "1.0".to_string()),
    ]),
    attrs: serde_json::json!({
        "custom_header": "value",
        "transaction_id": 12345,
    }),
};
```

### Status Code Mapping
```rust
fn map_status_to_protocol(status: u16) -> CustomStatus {
    match status {
        200..=299 => CustomStatus::Success,
        400..=499 => CustomStatus::ClientError,
        500..=599 => CustomStatus::ServerError,
        _ => CustomStatus::Unknown,
    }
}
```

## See Also

- [docs/router.md](router.md) - Pipeline architecture and flow
- [docs/architecture/diagrams.md](architecture/diagrams.md) - Architecture diagrams
- [docs/architecture/protocol-adapters.md](architecture/protocol-adapters.md) - Detailed design doc
- [src/adapters/http/](../src/adapters/http/) - HTTP adapter reference implementation
- [src/adapters/dimse/](../src/adapters/dimse/) - DIMSE adapter reference implementation
