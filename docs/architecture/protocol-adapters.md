# Protocol Adapter Architecture

## Overview

This document describes the architectural refactoring to make Harmony truly protocol-agnostic, treating HTTP, DIMSE, HL7 MLLP, and other protocols as first-class citizens through a unified adapter pattern.

## Problem Statement

### Current Architecture Issues

1. **HTTP-centric design**: Core routing and dispatching logic is tightly coupled to Axum/HTTP
2. **Protocol coupling**: `dispatcher.rs` takes `axum::Request` and returns `axum::Response`
3. **DIMSE as a hack**: DIMSE is launched as a side effect within HTTP router building
4. **Duplicate pipeline logic**: `pipeline_runner.rs` duplicates middleware/backend processing with broken semantics
5. **Return type bug**: DIMSE integration returns `RequestEnvelope` instead of `ResponseEnvelope`
6. **No extensibility**: Adding new protocols (HL7, SFTP, etc.) requires hacking around HTTP assumptions

### Current Flow (Broken)

```
HTTP Request
  → Axum Router
  → Dispatcher (HTTP-specific)
  → Middleware + Backends
  → Axum Response

DIMSE Request
  → SCP Listener (side effect of HTTP router build)
  → pipeline_runner.rs (duplicate/broken logic)
  → Returns RequestEnvelope (WRONG!)
  → ??? (cannot convert back to DIMSE properly)
```

## Target Architecture

### Design Principles

1. **Protocol agnostic core**: All pipeline logic works with `RequestEnvelope` → `ResponseEnvelope`
2. **Protocol adapters**: Each protocol is a first-class adapter that knows how to:
   - Listen for protocol-specific requests
   - Convert protocol → `ProtocolCtx` → `RequestEnvelope`
   - Execute common pipeline
   - Convert `ResponseEnvelope` → protocol response
3. **Single pipeline executor**: One source of truth for middleware + backend processing
4. **Zero configuration changes**: Existing configs work unchanged
5. **Future-ready**: Easy to add new protocols (HL7, SFTP, MQTT, etc.)

### Target Flow

```
Protocol Request (HTTP/DIMSE/HL7/etc.)
  ↓
Protocol Adapter
  ↓
ProtocolCtx + RequestEnvelope
  ↓
PipelineExecutor (common for all protocols)
  ├─ Endpoint service preprocessing
  ├─ Incoming middleware (left)
  ├─ Backend invocation
  ├─ Outgoing middleware (right)
  └─ Endpoint service postprocessing
  ↓
ResponseEnvelope
  ↓
Protocol Adapter
  ↓
Protocol Response (HTTP/DIMSE/HL7/etc.)
```

## Core Components

### 1. ProtocolCtx (Enhanced)

Carries protocol-specific metadata through the pipeline without coupling core logic to any protocol.

```rust
pub struct ProtocolCtx {
    pub protocol: Protocol,
    pub payload: Vec<u8>,
    pub meta: HashMap<String, String>,    // Simple key-value
    pub attrs: serde_json::Value,         // Rich structured data
}

// Example usage:
// HTTP: attrs = {"method": "POST", "headers": {...}, "cookies": {...}}
// DIMSE: attrs = {"calling_ae": "SCU", "called_ae": "SCP", "sop_class": "1.2.840..."}
```

**Typed accessors** (optional enhancement):
```rust
impl ProtocolCtx {
    pub fn get<T: FromAttrs>(&self) -> Option<T> {
        T::from_attrs(&self.attrs)
    }
}

// Usage:
let http_ctx: HttpCtx = ctx.get()?;
let dimse_ctx: DimseCtx = ctx.get()?;
```

### 2. PipelineExecutor

**Location**: `src/pipeline/executor.rs`

Single source of truth for pipeline execution. Protocol-agnostic, no Axum/HTTP types.

```rust
pub async fn execute(
    envelope: RequestEnvelope<Vec<u8>>,
    pipeline: &Pipeline,
    config: &Config,
    ctx: &ProtocolCtx,
) -> Result<ResponseEnvelope<Vec<u8>>, PipelineError> {
    // 1. Endpoint service preprocessing
    let envelope = service.endpoint_incoming_request(envelope, options).await?;
    
    // 2. Incoming middleware chain (left)
    let envelope = process_incoming_middleware(envelope, pipeline, config).await?;
    
    // 3. Backend invocation
    let response = process_backends(envelope, pipeline, config, service).await?;
    
    // 4. Outgoing middleware chain (right)
    let response = process_outgoing_middleware(response, pipeline, config).await?;
    
    // 5. Endpoint service postprocessing
    // (protocol-specific response shaping moved to adapters)
    
    Ok(response)
}
```

**Key features**:
- Extracts logic from `dispatcher.rs` but removes HTTP coupling
- Reuses existing middleware chain and backend processing
- Returns `ResponseEnvelope` (not `RequestEnvelope` like broken `pipeline_runner.rs`)
- Adds structured tracing with protocol tags

### 3. ProtocolAdapter Trait

**Location**: `src/adapters/mod.rs`

Defines the interface for protocol adapters.

```rust
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    /// Protocol type this adapter handles
    fn protocol(&self) -> Protocol;
    
    /// Start the adapter (listener, server, etc.)
    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> Result<JoinHandle<()>>;
    
    /// Adapter configuration summary (for logging/debugging)
    fn summary(&self) -> String;
}
```

### 4. Protocol Adapters

#### HttpAdapter

**Location**: `src/adapters/http/mod.rs`

Wraps Axum and handles HTTP protocol I/O.

```rust
pub struct HttpAdapter {
    network_name: String,
    bind_addr: SocketAddr,
}

impl HttpAdapter {
    async fn start(&self, config: Arc<Config>, shutdown: CancellationToken) -> Result<JoinHandle<()>> {
        let router = self.build_router(&config)?;
        
        tokio::spawn(async move {
            let listener = TcpListener::bind(bind_addr).await?;
            
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled())
                .await
        })
    }
    
    fn build_router(&self, config: &Config) -> Router {
        // For each pipeline in this network:
        for pipeline in pipelines {
            for endpoint in &pipeline.endpoints {
                // Register route handler:
                app = app.route(path, handler(|req: Request| async {
                    // 1. Convert HTTP Request → ProtocolCtx
                    let ctx = http_request_to_protocol_ctx(req)?;
                    
                    // 2. Build envelope via service
                    let envelope = service.build_protocol_envelope(ctx, options)?;
                    
                    // 3. Execute pipeline
                    let response = PipelineExecutor::execute(envelope, pipeline, config, &ctx).await?;
                    
                    // 4. Convert ResponseEnvelope → HTTP Response
                    let http_response = response_envelope_to_http(response, &ctx)?;
                    
                    Ok(http_response)
                }));
            }
        }
        
        app
    }
}
```

**Key features**:
- Moves route construction logic from `dispatcher.rs`
- Preserves route conflict detection
- Converts between Axum types and protocol-agnostic envelopes
- Calls `PipelineExecutor` instead of inline processing

#### DimseAdapter

**Location**: `src/adapters/dimse/mod.rs`

Wraps DIMSE SCP and handles DICOM protocol I/O.

```rust
pub struct DimseAdapter {
    endpoint_name: String,
    pipeline_name: String,
    local_aet: String,
    bind_addr: IpAddr,
    port: u16,
}

impl DimseAdapter {
    async fn start(&self, config: Arc<Config>, shutdown: CancellationToken) -> Result<JoinHandle<()>> {
        let dimse_config = DimseConfig {
            local_aet: self.local_aet.clone(),
            bind_addr: self.bind_addr,
            port: self.port,
            enable_find: true,
            enable_move: true,
            enable_store: true,
            ..Default::default()
        };
        
        let provider = Arc::new(PipelineQueryProvider {
            pipeline: self.pipeline_name.clone(),
            endpoint: self.endpoint_name.clone(),
            config: config.clone(),
        });
        
        tokio::spawn(async move {
            let scp = DimseScp::new(dimse_config, provider);
            
            tokio::select! {
                result = scp.run() => {
                    if let Err(e) = result {
                        tracing::error!("DIMSE SCP failed: {}", e);
                    }
                }
                _ = shutdown.cancelled() => {
                    tracing::info!("DIMSE SCP shutting down");
                }
            }
        })
    }
}

// QueryProvider implementation
impl QueryProvider for PipelineQueryProvider {
    async fn find(&self, level: QueryLevel, params: &HashMap<String, String>, max: u32) 
        -> Result<Vec<DatasetStream>> 
    {
        // 1. Build ProtocolCtx
        let mut meta = HashMap::new();
        meta.insert("operation".into(), "C-FIND".into());
        meta.insert("query_level".into(), format!("{}", level));
        
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&self.build_identifier_json(params))?,
            meta,
            attrs: serde_json::json!({
                "query_level": level,
                "max_results": max,
            }),
        };
        
        // 2. Build envelope via service
        let service = endpoint.resolve_service()?;
        let envelope = service.build_protocol_envelope(ctx, options)?;
        
        // 3. Execute pipeline
        let response = PipelineExecutor::execute(envelope, pipeline, &config, &ctx).await?;
        
        // 4. Convert ResponseEnvelope → DIMSE C-FIND responses
        let datasets = self.response_to_find_datasets(response)?;
        
        Ok(datasets)
    }
    
    async fn store(&self, dataset: DatasetStream) -> Result<()> {
        // Similar pattern for C-STORE
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: dataset.to_bytes()?,
            meta: hashmap!{"operation".into() => "C-STORE".into()},
            attrs: serde_json::json!({}),
        };
        
        let envelope = service.build_protocol_envelope(ctx, options)?;
        let response = PipelineExecutor::execute(envelope, pipeline, &config, &ctx).await?;
        
        // Check response status
        if response.response_details.status >= 400 {
            return Err(DimseError::operation_failed("Store failed"));
        }
        
        Ok(())
    }
}
```

**Key features**:
- Replaces `scp_launcher.rs` and `pipeline_query_provider.rs` logic
- Converts DIMSE operations → `ProtocolCtx` → `RequestEnvelope`
- Calls `PipelineExecutor` (not broken `pipeline_runner.rs`)
- Returns `ResponseEnvelope` and converts to DIMSE responses correctly
- Handles C-FIND multi-result streaming
- Maps pipeline errors → DIMSE status codes

### 5. Adapter Orchestration

**Location**: `src/lib.rs` (enhanced `run()` function)

```rust
pub async fn run(config: Config) {
    let config = Arc::new(config);
    let shutdown = CancellationToken::new();
    
    // Registry of adapter handles
    let mut adapter_handles = Vec::new();
    
    // For each network, determine which adapters to start
    for (network_name, network) in &config.network {
        // HTTP adapter
        if network.http.enabled {
            let adapter = HttpAdapter::new(network_name.clone(), network.clone());
            let handle = adapter.start(config.clone(), shutdown.clone()).await?;
            adapter_handles.push(handle);
        }
        
        // DIMSE adapters (per pipeline with DIMSE endpoints)
        for (pipeline_name, pipeline) in &config.pipelines {
            if !pipeline.networks.contains(network_name) {
                continue;
            }
            
            for endpoint_name in &pipeline.endpoints {
                let endpoint = config.endpoints.get(endpoint_name)?;
                
                if endpoint.service.eq_ignore_ascii_case("dicom") {
                    let adapter = DimseAdapter::new(
                        endpoint_name.clone(),
                        pipeline_name.clone(),
                        endpoint.options.clone(),
                    );
                    let handle = adapter.start(config.clone(), shutdown.clone()).await?;
                    adapter_handles.push(handle);
                }
            }
        }
    }
    
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    shutdown.cancel();
    
    // Gracefully await all adapters
    for handle in adapter_handles {
        let _ = tokio::time::timeout(Duration::from_secs(30), handle).await;
    }
}
```

## Service Layer Changes

### Protocol-Agnostic Response Hook

Add new trait method to `ServiceType`:

```rust
#[async_trait]
pub trait ServiceType: ServiceHandler<Value> {
    // Existing methods...
    
    /// Protocol-agnostic response postprocessing
    /// Allows services to shape responses based on ProtocolCtx
    async fn endpoint_outgoing_protocol(
        &self,
        response: ResponseEnvelope<Vec<u8>>,
        ctx: &ProtocolCtx,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // Default: no-op
        Ok(response)
    }
}
```

**Usage example** (DicomwebEndpoint):
```rust
async fn endpoint_outgoing_protocol(
    &self,
    mut response: ResponseEnvelope<Vec<u8>>,
    ctx: &ProtocolCtx,
) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
    match ctx.protocol {
        Protocol::Http => {
            // Set DICOMweb-specific headers
            response.response_details.headers.insert(
                "content-type".into(),
                "application/dicom+json".into(),
            );
        }
        Protocol::Dimse => {
            // Ensure dataset is properly formatted for DIMSE transport
            // (adapter will handle final encoding)
        }
        _ => {}
    }
    Ok(response)
}
```

## Migration Path

### Phase 1: Foundation (No behavior change)
- Create `src/pipeline/` and `src/adapters/` modules
- Implement `PipelineExecutor` by extracting from dispatcher
- Define `ProtocolAdapter` trait
- **Tests**: Existing tests still pass

### Phase 2: HTTP Adapter
- Implement `HttpAdapter`
- Move route building from dispatcher
- Update `lib.rs` to start `HttpAdapter`
- Keep old dispatcher paths temporarily for compatibility
- **Tests**: HTTP routes work via adapter

### Phase 3: DIMSE Adapter
- Implement `DimseAdapter`
- Replace `pipeline_query_provider` usage of `pipeline_runner`
- Move SCP launching from `scp_launcher.rs`
- **Tests**: DIMSE C-FIND/C-STORE work via adapter

### Phase 4: Cleanup
- Delete `pipeline_runner.rs`
- Delete or deprecate `scp_launcher.rs`
- Remove old dispatcher pipeline execution code
- **Tests**: All tests pass, no duplicate logic

### Phase 5: Documentation & Polish
- Update docs
- Add adapter examples
- Performance and security review

## Benefits

1. **True protocol abstraction**: HTTP and DIMSE are peers, not special cases
2. **No duplicate logic**: One `PipelineExecutor` for all protocols
3. **Correct return types**: DIMSE properly returns `ResponseEnvelope`
4. **Easy extensibility**: Adding HL7 MLLP is just a new adapter
5. **Better separation of concerns**: Protocol I/O is separate from business logic
6. **No config changes**: Existing configurations work unchanged
7. **Better testing**: Can test pipeline executor in isolation
8. **Better observability**: Consistent tracing across all protocols

## Future Protocols

With this architecture, adding new protocols is straightforward:

### Example: HL7 MLLP Adapter

```rust
pub struct Hl7MllpAdapter {
    bind_addr: SocketAddr,
}

impl ProtocolAdapter for Hl7MllpAdapter {
    async fn start(&self, config: Arc<Config>, shutdown: CancellationToken) -> Result<JoinHandle<()>> {
        tokio::spawn(async move {
            let listener = TcpListener::bind(bind_addr).await?;
            
            loop {
                tokio::select! {
                    accept = listener.accept() => {
                        let (stream, _) = accept?;
                        tokio::spawn(handle_hl7_connection(stream, config.clone()));
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        })
    }
}

async fn handle_hl7_connection(mut stream: TcpStream, config: Arc<Config>) {
    // Read HL7 message with MLLP framing (<SB>message<EB><CR>)
    let hl7_message = read_mllp_message(&mut stream).await?;
    
    // Build ProtocolCtx
    let ctx = ProtocolCtx {
        protocol: Protocol::Hl7V2Mllp,
        payload: hl7_message.as_bytes().to_vec(),
        meta: hashmap!{
            "message_type".into() => extract_message_type(&hl7_message)?,
        },
        attrs: serde_json::json!({}),
    };
    
    // Build envelope and execute pipeline
    let envelope = service.build_protocol_envelope(ctx, options)?;
    let response = PipelineExecutor::execute(envelope, pipeline, &config, &ctx).await?;
    
    // Convert response to HL7 ACK and send
    let ack = build_hl7_ack(&response)?;
    write_mllp_message(&mut stream, &ack).await?;
}
```

## Implementation Checklist

See the TODO list for detailed phase-by-phase implementation plan covering:
- ✅ Foundation modules and traits
- ✅ PipelineExecutor extraction
- ✅ HttpAdapter implementation
- ✅ DimseAdapter implementation
- ✅ Service layer updates
- ✅ Orchestration changes
- ✅ Cleanup and deletions
- ✅ Testing strategy
- ✅ Documentation updates

## Acceptance Criteria

- [ ] HTTP requests flow through `HttpAdapter` → `PipelineExecutor` → `HttpAdapter`
- [ ] DIMSE requests flow through `DimseAdapter` → `PipelineExecutor` → `DimseAdapter`
- [ ] No duplicate pipeline execution logic (only one `PipelineExecutor`)
- [ ] `pipeline_runner.rs` deleted
- [ ] `scp_launcher.rs` moved to `DimseAdapter` or deleted
- [ ] All existing tests pass
- [ ] New adapter-specific tests pass
- [ ] Configuration format unchanged
- [ ] Documentation updated with adapter pattern
- [ ] Performance benchmarks show no regression
- [ ] Security review confirms PHI safety unchanged

## References

- [Current Router Documentation](../router.md)
- [Envelope Documentation](../envelope.md)
- [DIMSE Integration Documentation](../dimse-integration.md)
- [Project Guidelines](.junie/guidelines.md)
