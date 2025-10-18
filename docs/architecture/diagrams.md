# Architecture Diagrams

## Current Architecture (Before Refactoring)

### HTTP Request Flow
```
┌─────────────────────────────────────────────────────────────┐
│                       HTTP Request                          │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                     Axum Router                             │
│                   (HTTP-specific)                           │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                    Dispatcher.rs                            │
│              (Tightly coupled to HTTP)                      │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 1. Convert HTTP Request → RequestEnvelope            │  │
│  │ 2. Process incoming middleware (left)                │  │
│  │ 3. Process backends                                  │  │
│  │ 4. Process outgoing middleware (right)               │  │
│  │ 5. Convert ResponseEnvelope → HTTP Response          │  │
│  └──────────────────────────────────────────────────────┘  │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                    HTTP Response                            │
└─────────────────────────────────────────────────────────────┘
```

### DIMSE Request Flow (Current - BROKEN)
```
┌─────────────────────────────────────────────────────────────┐
│                   DIMSE Request                             │
│              (C-FIND / C-MOVE / C-STORE)                    │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                    DIMSE SCP                                │
│         (Launched as side effect of                         │
│          HTTP router building)                              │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│              pipeline_query_provider.rs                     │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│               pipeline_runner.rs                            │
│            (DUPLICATE & BROKEN LOGIC)                       │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ ❌ Returns RequestEnvelope (WRONG!)                  │  │
│  │ ❌ Cannot convert back to DIMSE properly             │  │
│  │ ❌ Duplicates dispatcher middleware logic            │  │
│  │ ❌ Backend processing broken                         │  │
│  └──────────────────────────────────────────────────────┘  │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                 ??? BROKEN ???                              │
│    (Cannot properly convert RequestEnvelope                 │
│          back to DIMSE response)                            │
└─────────────────────────────────────────────────────────────┘
```

### Problems with Current Architecture
```
╔═══════════════════════════════════════════════════════════╗
║                    ISSUES                                 ║
╠═══════════════════════════════════════════════════════════╣
║ 1. HTTP and DIMSE use DIFFERENT pipeline execution paths ║
║ 2. pipeline_runner.rs DUPLICATES dispatcher logic         ║
║ 3. DIMSE returns RequestEnvelope (should be Response!)    ║
║ 4. No way to add HL7, SFTP, etc. without more hacks      ║
║ 5. Protocol I/O mixed with business logic                 ║
╚═══════════════════════════════════════════════════════════╝
```

---

## Target Architecture (After Refactoring)

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

## Module Structure

### Before
```
src/
├── router/
│   ├── mod.rs
│   ├── dispatcher.rs          ← HTTP-specific, inline pipeline exec
│   ├── pipeline_runner.rs     ← DUPLICATE & BROKEN 🗑️
│   ├── scp_launcher.rs        ← Side effect launcher 🗑️
│   └── route_config.rs
├── integrations/
│   └── dimse/
│       ├── mod.rs
│       └── pipeline_query_provider.rs  ← Uses broken pipeline_runner
└── models/
    └── services/
        └── services.rs        ← HTTP-specific response handling
```

### After
```
src/
├── pipeline/                  ← NEW: Protocol-agnostic core
│   ├── mod.rs
│   ├── executor.rs           ← SINGLE SOURCE OF TRUTH ⭐
│   └── tests.rs
│
├── adapters/                  ← NEW: Protocol-specific I/O
│   ├── mod.rs                ← ProtocolAdapter trait
│   ├── http/
│   │   ├── mod.rs           ← HttpAdapter
│   │   └── router.rs
│   ├── dimse/
│   │   ├── mod.rs           ← DimseAdapter
│   │   └── query_provider.rs
│   └── hl7_mllp/            ← Future: Easy to add
│       └── mod.rs
│
├── router/
│   ├── mod.rs               ← Thinned, delegates to adapters
│   ├── dispatcher.rs        ← Thinned, no pipeline exec
│   └── route_config.rs
│
├── integrations/            ← May be deprecated/moved
│   └── dimse/
│       └── ...              ← Moved to adapters/dimse
│
└── models/
    └── services/
        └── services.rs      ← Add endpoint_outgoing_protocol()
```

---

## Data Flow Comparison

### Current (Broken)
```
HTTP:  Request → Dispatcher → Middleware → Backend → Response ✅
DIMSE: Request → SCP → pipeline_runner → ❌ Wrong type → ??? ❌

Different paths, duplicate logic, broken DIMSE
```

### Target (Fixed)
```
HTTP:  Request → HttpAdapter → PipelineExecutor → HttpAdapter → Response ✅
DIMSE: Request → DimseAdapter → PipelineExecutor → DimseAdapter → Response ✅
HL7:   Request → Hl7Adapter → PipelineExecutor → Hl7Adapter → Response ✅

Same path, single logic, all protocols work correctly
```

---

## Adding New Protocols

### Current Architecture (Hard)
```
❌ To add HL7 MLLP:
1. Hack around HTTP router
2. Duplicate pipeline_runner logic (broken)
3. Try to work around Axum types
4. Fight with type mismatches
5. Give up or create more hacks
```

### Target Architecture (Easy)
```
✅ To add HL7 MLLP:
1. Create Hl7MllpAdapter
2. Implement protocol I/O (listen, parse, format)
3. Convert HL7 ↔ ProtocolCtx ↔ Envelope
4. Call PipelineExecutor (already works!)
5. Done! ✨
```

---

## Orchestration Flow

### Current (HTTP-only spawning)
```rust
// src/lib.rs
pub async fn run(config: Config) {
    for (network_name, network) in &config.network {
        tokio::spawn(async move {
            // Only HTTP server spawned
            let router = build_network_router(config, network_name).await;
            serve(listener, router).await
        });
    }
}
```

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

---

## Summary

The target architecture treats all protocols as first-class citizens through a unified adapter pattern, with a single protocol-agnostic pipeline executor that handles all middleware and backend processing. This eliminates duplicate logic, fixes the DIMSE return type bug, and makes it trivial to add new protocols.
