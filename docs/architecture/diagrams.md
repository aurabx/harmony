# Architecture Diagrams
### Unified Request Flow (All Protocols)
```
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│  HTTP Request    │  │  DIMSE Request   │  │  HL7 Request     │
└────────┬─────────┘  └────────┬─────────┘  └────────┬─────────┘
         │                     │                      │
         ▼                     ▼                      ▼
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│  HttpAdapter     │  │  DimseAdapter    │  │  Hl7MllpAdapter  │
│                  │  │                  │  │                  │
│ • Axum Server    │  │ • DIMSE SCP      │  │ • MLLP Listener  │
│ • Route matching │  │ • C-FIND/MOVE    │  │ • HL7 Parser     │
│ • HTTP I/O       │  │ • C-STORE        │  │ • HL7 I/O        │
└────────┬─────────┘  └────────┬─────────┘  └────────┬─────────┘
         │                     │                      │
         │   Protocol-specific conversion             │
         │                     │                      │
         ▼                     ▼                      ▼
┌─────────────────────────────────────────────────────────────┐
│                      ProtocolCtx                            │
│  {                                                          │
│    protocol: HTTP | DIMSE | HL7 | ...,                     │
│    payload: Vec<u8>,                                        │
│    meta: HashMap<String, String>,                           │
│    attrs: serde_json::Value                                 │
│  }                                                          │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                   RequestEnvelope                           │
│              (Protocol-agnostic)                            │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
╔═════════════════════════════════════════════════════════════╗
║              PipelineExecutor (SINGLE SOURCE)               ║
║                (Protocol-agnostic core)                     ║
╠═════════════════════════════════════════════════════════════╣
║                                                             ║
║  ┌─────────────────────────────────────────────────────┐  ║
║  │ 1. Endpoint Service Preprocessing                   │  ║
║  │    service.endpoint_incoming_request()              │  ║
║  └─────────────────────────────────────────────────────┘  ║
║                        │                                    ║
║  ┌─────────────────────▼───────────────────────────────┐  ║
║  │ 2. Incoming Middleware Chain (left)                 │  ║
║  │    • JWT Auth                                       │  ║
║  │    • Transforms                                     │  ║
║  │    • Custom middleware                              │  ║
║  └─────────────────────────────────────────────────────┘  ║
║                        │                                    ║
║  ┌─────────────────────▼───────────────────────────────┐  ║
║  │ 3. Backend Invocation                               │  ║
║  │    • HTTP backend                                   │  ║
║  │    • FHIR server                                    │  ║
║  │    • DICOM PACS                                     │  ║
║  │    • Custom backends                                │  ║
║  │                                                     │  ║
║  │    Returns: ResponseEnvelope ✅                     │  ║
║  └─────────────────────────────────────────────────────┘  ║
║                        │                                    ║
║  ┌─────────────────────▼───────────────────────────────┐  ║
║  │ 4. Outgoing Middleware Chain (right)                │  ║
║  │    • Response transforms                            │  ║
║  │    • Logging                                        │  ║
║  │    • Header injection                               │  ║
║  └─────────────────────────────────────────────────────┘  ║
║                        │                                    ║
║  ┌─────────────────────▼───────────────────────────────┐  ║
║  │ 5. Endpoint Service Postprocessing                  │  ║
║  │    service.endpoint_outgoing_protocol()             │  ║
║  └─────────────────────────────────────────────────────┘  ║
║                                                             ║
╚════════════════════════┬════════════════════════════════════╝
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                  ResponseEnvelope                           │
│              (Protocol-agnostic)                            │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                    ProtocolCtx                              │
└────────────────────────┬────────────────────────────────────┘
         │                     │                      │
         │   Protocol-specific conversion             │
         │                     │                      │
         ▼                     ▼                      ▼
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│  HttpAdapter     │  │  DimseAdapter    │  │  Hl7MllpAdapter  │
└────────┬─────────┘  └────────┬─────────┘  └────────┬─────────┘
         │                     │                      │
         ▼                     ▼                      ▼
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│  HTTP Response   │  │  DIMSE Response  │  │  HL7 ACK         │
└──────────────────┘  └──────────────────┘  └──────────────────┘
```

### Benefits Visualization
```
╔═══════════════════════════════════════════════════════════════╗
║                    TARGET BENEFITS                            ║
╠═══════════════════════════════════════════════════════════════╣
║                                                               ║
║  ✅ Single PipelineExecutor                                  ║
║     └─ No duplicate logic                                    ║
║     └─ One source of truth                                   ║
║                                                               ║
║  ✅ Protocol Abstraction                                     ║
║     └─ HTTP and DIMSE are peers                              ║
║     └─ Easy to add HL7, SFTP, MQTT, etc.                     ║
║                                                               ║
║  ✅ Correct Return Types                                     ║
║     └─ DIMSE properly returns ResponseEnvelope               ║
║     └─ Can convert back to protocol responses                ║
║                                                               ║
║  ✅ Separation of Concerns                                   ║
║     └─ Protocol I/O in adapters                              ║
║     └─ Business logic in pipeline                            ║
║                                                               ║
║  ✅ Better Testing                                           ║
║     └─ Test pipeline in isolation                            ║
║     └─ Test adapters independently                           ║
║                                                               ║
║  ✅ No Config Changes                                        ║
║     └─ Existing configurations work unchanged                ║
║                                                               ║
╚═══════════════════════════════════════════════════════════════╝
```

---
## Data Flow
```
HTTP:  Request → HttpAdapter → PipelineExecutor → HttpAdapter → Response ✅
DIMSE: Request → DimseAdapter → PipelineExecutor → DimseAdapter → Response ✅
HL7:   Request → Hl7Adapter → PipelineExecutor → Hl7Adapter → Response ✅

Same path, single logic, all protocols work correctly
```


## Orchestration Flow

### Target (Protocol-aware spawning)
```rust
// src/lib.rs
pub async fn run(config: Config) {
    let mut adapters = Vec::new();
    
    for (network_name, network) in &config.network {
        // HTTP adapter
        if has_http_endpoints(network) {
            let adapter = HttpAdapter::new(network);
            adapters.push(adapter.start(config, shutdown));
        }
        
        // DIMSE adapter
        if has_dimse_endpoints(network) {
            let adapter = DimseAdapter::new(network);
            adapters.push(adapter.start(config, shutdown));
        }
        
        // HL7 adapter
        if has_hl7_endpoints(network) {
            let adapter = Hl7MllpAdapter::new(network);
            adapters.push(adapter.start(config, shutdown));
        }
    }
    
    // Wait for shutdown
    signal::ctrl_c().await;
    shutdown.cancel();
    
    // Graceful shutdown all adapters
    for handle in adapters {
        handle.await;
    }
}
```