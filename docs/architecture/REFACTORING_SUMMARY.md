# Protocol Adapter Refactoring - Executive Summary

## The Problem

Harmony currently treats HTTP as the primary protocol with DIMSE bolted on as an afterthought:

```
❌ Current: HTTP-centric with DIMSE as a hack
├─ HTTP uses dispatcher.rs (correct pipeline flow)
└─ DIMSE uses pipeline_runner.rs (broken duplicate logic)
   └─ Returns RequestEnvelope instead of ResponseEnvelope ❌
   └─ Cannot properly convert back to DIMSE responses ❌
```

**Issues:**
- Duplicate pipeline execution logic (dispatcher vs pipeline_runner)
- DIMSE returns wrong type (RequestEnvelope instead of ResponseEnvelope)
- HTTP and DIMSE use different execution paths
- Adding new protocols (HL7, SFTP) requires hacking around HTTP assumptions
- No clean separation between protocol I/O and business logic

## The Solution

Make protocols first-class citizens through a unified adapter pattern:

```
✅ Target: Protocol-agnostic core with protocol adapters

Protocol Request (HTTP/DIMSE/HL7/...)
  ↓
ProtocolAdapter (HTTP/DIMSE/HL7/...)
  ↓ converts to
ProtocolCtx + RequestEnvelope
  ↓
PipelineExecutor (SINGLE SOURCE OF TRUTH)
  ├─ Endpoint preprocessing
  ├─ Incoming middleware (left)
  ├─ Backend invocation
  ├─ Outgoing middleware (right)
  └─ Endpoint postprocessing
  ↓
ResponseEnvelope
  ↓ converts back
ProtocolAdapter
  ↓
Protocol Response (HTTP/DIMSE/HL7/...)
```

## Key Components

### 1. PipelineExecutor
- **Single source of truth** for all pipeline processing
- Protocol-agnostic (no Axum/HTTP types)
- Extracted from dispatcher.rs but made reusable
- Returns ResponseEnvelope (not RequestEnvelope!)

### 2. ProtocolAdapter Trait
```rust
trait ProtocolAdapter {
    fn protocol(&self) -> Protocol;
    async fn start(config, shutdown) -> JoinHandle<()>;
}
```

### 3. HttpAdapter
- Wraps Axum HTTP server
- Converts HTTP ↔ ProtocolCtx ↔ Envelope
- Calls PipelineExecutor

### 4. DimseAdapter
- Wraps DIMSE SCP
- Converts DIMSE ↔ ProtocolCtx ↔ Envelope
- Calls PipelineExecutor (not broken pipeline_runner!)
- Properly returns ResponseEnvelope

### 5. Orchestrator
- `src/lib.rs` spawns adapters per network/pipeline
- HTTP adapter for HTTP endpoints
- DIMSE adapter for DICOM endpoints
- Future: HL7 MLLP, SFTP, etc.

## Implementation Phases

### Phase 0: Baseline ✅
- Branch: `feature/protocol-adapters`
- Ensure tests pass
- No config changes

### Phase 1: Foundation (PR1)
- Create `src/pipeline/executor.rs`
- Create `src/adapters/mod.rs`
- Extract PipelineExecutor from dispatcher
- Define ProtocolAdapter trait
- **No behavior change yet**

### Phase 2: HTTP Adapter (PR2)
- Implement `src/adapters/http/mod.rs`
- Move route building from dispatcher
- Update `lib.rs` to spawn HttpAdapter
- Keep old paths for compatibility

### Phase 3: DIMSE Adapter (PR3-4)
- Implement `src/adapters/dimse/mod.rs`
- Replace pipeline_runner usage in pipeline_query_provider
- Fix return type: ResponseEnvelope ✅
- Implement C-FIND/C-STORE/C-MOVE properly

### Phase 4: Cleanup (PR5-6)
- **Delete** `src/router/pipeline_runner.rs` 🗑️
- **Delete** `src/router/scp_launcher.rs` 🗑️
- Remove duplicate logic from dispatcher
- Update documentation

### Phase 5: Testing & Polish
- Unit tests for PipelineExecutor
- Integration tests for adapters
- Performance benchmarks
- Security review

## Benefits

✅ **True protocol abstraction**: HTTP and DIMSE are peers  
✅ **No duplicate logic**: One PipelineExecutor for all  
✅ **Correct types**: DIMSE returns ResponseEnvelope  
✅ **Easy extensibility**: New protocols = new adapter  
✅ **Better testing**: Test pipeline in isolation  
✅ **Better observability**: Consistent tracing  
✅ **No config changes**: Existing configs work unchanged  

## Files Changed

### New Files
- `src/pipeline/mod.rs`
- `src/pipeline/executor.rs`
- `src/adapters/mod.rs`
- `src/adapters/http/mod.rs`
- `src/adapters/dimse/mod.rs`
- `docs/architecture/protocol-adapters.md`
- `docs/adapters.md`

### Modified Files
- `src/lib.rs` - Spawn adapters instead of HTTP-only
- `src/router/dispatcher.rs` - Thinned, no pipeline execution
- `src/models/services/services.rs` - Add endpoint_outgoing_protocol()
- `src/integrations/dimse/pipeline_query_provider.rs` - Use PipelineExecutor

### Deleted Files
- `src/router/pipeline_runner.rs` 🗑️
- `src/router/scp_launcher.rs` 🗑️

## Migration Safety

- ✅ Configuration format unchanged
- ✅ Existing tests pass throughout
- ✅ Backward compatibility maintained during transition
- ✅ Multiple small PRs (not one giant change)
- ✅ Each phase is independently testable

## Future Protocols

Adding HL7 MLLP becomes simple:

```rust
// src/adapters/hl7_mllp/mod.rs
pub struct Hl7MllpAdapter { ... }

impl ProtocolAdapter for Hl7MllpAdapter {
    async fn start(&self, config, shutdown) -> JoinHandle<()> {
        // Listen for MLLP connections
        // Convert HL7 message → ProtocolCtx → RequestEnvelope
        // Call PipelineExecutor::execute()
        // Convert ResponseEnvelope → HL7 ACK
    }
}
```

Same pattern for SFTP, MQTT, Kafka, WebRTC, etc.

## Success Criteria

- [ ] HTTP: HttpAdapter → PipelineExecutor → HttpAdapter
- [ ] DIMSE: DimseAdapter → PipelineExecutor → DimseAdapter
- [ ] Only one PipelineExecutor (no duplicates)
- [ ] pipeline_runner.rs deleted
- [ ] scp_launcher.rs deleted
- [ ] All tests pass
- [ ] No config changes
- [ ] Documentation complete

## Next Steps

1. Review this plan with team
2. Create feature branch: `feature/protocol-adapters`
3. Start Phase 1: Foundation (PR1)
4. Iterate through phases with small PRs
5. Final review and merge

## Documentation

- **Architecture**: [docs/architecture/protocol-adapters.md](./protocol-adapters.md)
- **Implementation Plan**: See TODO list (26 phases)
- **Current Router**: [docs/router.md](../router.md)
- **Project Guidelines**: [warp.md](../../warp.md)

---

**Questions?** Refer to the detailed architecture document or reach out to the team.
