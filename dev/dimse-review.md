# DIMSE Subsystem Code Review

**Date:** 2025-12-22  
**Branch:** `feat/aura-2208-dicom-to-dicomweb`  
**Status:** Review Complete

## Overview

The rebuilt DIMSE subsystem provides DICOM DIMSE protocol support with clear separation of concerns:

- **`crates/dimse/`**: Core DIMSE protocol implementation (SCU/SCP)
- **`src/adapters/dimse/`**: Protocol adapter integrating dimse crate with Harmony pipelines

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Harmony Proxy                                │
├─────────────────────────────────────────────────────────────────┤
│  DimseAdapter (src/adapters/dimse/mod.rs)                        │
│  └─> PipelineQueryProvider (query_provider.rs)                   │
│      └─> Executes pipeline via PipelineExecutor                  │
├─────────────────────────────────────────────────────────────────┤
│  DicomScpEndpoint (models/services/types/dicom_scp.rs)           │
│  DicomScuBackend  (models/services/types/dicom.rs)               │
├─────────────────────────────────────────────────────────────────┤
│  dimse crate (crates/dimse/)                                     │
│  ├─> DimseScp (SCP: listens for connections)                     │
│  │   └─> QueryProvider trait (find/locate/get/store)             │
│  └─> DimseScu (SCU: makes outbound connections)                  │
└─────────────────────────────────────────────────────────────────┘
```

## Key Components

### dimse crate (`crates/dimse/`)

| File | Purpose |
|------|---------|
| `lib.rs` | Public exports and re-exports |
| `config.rs` | `DimseConfig`, `RemoteNode`, `TlsConfig` |
| `types.rs` | `DatasetStream`, `FindQuery`, `MoveQuery`, `GetQuery`, `QueryLevel`, `DimseStatus` |
| `error.rs` | `DimseError` enum with helper constructors |
| `router.rs` | Internal router for decoupling DIMSE from HTTP layer |
| `scp/mod.rs` | `DimseScp`, `QueryProvider` trait |
| `scp/association.rs` | Association lifecycle management |
| `scp/pdu_handler.rs` | PDU parsing and command dispatch |
| `scp/commands/*.rs` | Individual command handlers (echo, find, get, move, store) |
| `scu/mod.rs` | `DimseScu` client implementation |

### Harmony Adapter (`src/adapters/dimse/`)

| File | Purpose |
|------|---------|
| `mod.rs` | `DimseAdapter` implementing `ProtocolAdapter` trait |
| `query_provider.rs` | `PipelineQueryProvider` - bridges DIMSE to pipeline system |
| `status_mapper.rs` | HTTP/Pipeline error to DIMSE status code mapping |

## Strengths

1. **Clean Protocol Abstraction**: The `QueryProvider` trait provides a clean interface for SCP operations with `find`, `locate`, `get`, and `store` methods.

2. **Pipeline Integration**: `PipelineQueryProvider` bridges DIMSE operations to Harmony's pipeline system, converting DICOM queries to protocol contexts and executing through `PipelineExecutor`.

3. **Robust Status Mapping**: `status_mapper.rs` provides bidirectional mapping between HTTP status codes, pipeline errors, and DIMSE status codes.

4. **Flexible SCP Options**: Supports both internal SCP (using the dimse crate) and DCMTK storescp fallback for persistent store operations.

5. **Duplicate Prevention**: The `STARTED_SCP` registry prevents starting duplicate listeners on the same AET/port combination.

6. **Async-Native**: Built on tokio with proper async patterns throughout.

---

## Issues & Recommendations

### HIGH Priority

#### 1. DatasetStream JSON Conversion (Potential Bug)

**Location:** `src/adapters/dimse/query_provider.rs:228-234`

**Problem:** The `json_to_dataset` method serializes JSON back to bytes for `DatasetStream::from_bytes`, but `DatasetStream::from_bytes` creates a Memory variant that expects DICOM bytes, not JSON.

```rust
let json_bytes = serde_json::to_vec(json).map_err(|e| {
    DimseError::operation_failed(format!("Failed to serialize JSON: {}", e))
})?;

let dataset = DatasetStream::from_bytes(json_bytes.into());
```

If these bytes are later parsed as DICOM (via `to_object()`), parsing will fail because JSON is not valid DICOM.

**Recommendation:** Either:
1. Convert JSON to actual DICOM objects before creating `DatasetStream`
2. Add a new `DatasetStream::Json` variant for JSON data
3. Use `DatasetStream::Object` after parsing DICOM JSON to `InMemDicomObject`

---

### MEDIUM Priority

#### 2. VR Mapping Hardcoded

**Location:** `src/adapters/dimse/query_provider.rs:49-54`

**Problem:** VR (Value Representation) mapping is hardcoded for only 3 tags, defaulting to "UN" (Unknown) for all others.

```rust
let vr = match tag.as_str() {
    "00100010" => "PN",
    "00100020" => "LO",
    "00080020" => "DA",
    _ => "UN",
};
```

**Recommendation:** Use a VR lookup from a DICOM dictionary, or expand the mapping to include common query tags:
- `0020000D` (StudyInstanceUID) → UI
- `0020000E` (SeriesInstanceUID) → UI
- `00080060` (Modality) → CS
- `00080050` (AccessionNumber) → SH

#### 3. Error Handling in DCMTK Fallback

**Location:** `src/adapters/dimse/mod.rs:366-390`

**Problem:** The DCMTK storescp process doesn't respect the `shutdown` cancellation token - it will run until completion or error.

**Recommendation:** 
- Use `child.kill()` on shutdown signal
- Or spawn with proper signal handling via `tokio::select!`

#### 4. Query Level Inference Uses Magic Strings

**Location:** `src/models/services/types/dicom.rs:467-492`

**Problem:** Tag identifiers like `"00080018"` are magic strings scattered throughout.

**Recommendation:** Extract to constants:
```rust
const TAG_SOP_INSTANCE_UID: &str = "00080018";
const TAG_SERIES_INSTANCE_UID: &str = "0020000E";
const TAG_STUDY_INSTANCE_UID: &str = "0020000D";
```

---

### LOW Priority

#### 5. Dead Code - CURRENT_STORE_DIR

**Location:** `src/adapters/dimse/query_provider.rs:16-25`

**Problem:** `CURRENT_STORE_DIR` static is defined but `set_current_store_dir` is commented out. The `store` method uses `get_storage()` globally instead.

**Recommendation:** Remove the unused static and associated functions.

#### 6. Blocking I/O in Async Context

**Location:** `src/models/services/types/dicom.rs:724, 808`

**Problem:** Uses `std::fs::read_dir` and `std::fs::rename` in async context.

**Recommendation:** Use `tokio::fs` equivalents or `tokio::task::spawn_blocking`.

#### 7. Missing Test Coverage

**Location:** `src/adapters/dimse/query_provider.rs`

**Problem:** The `run` method and `response_to_datasets` method lack integration tests.

**Recommendation:** Add tests for:
- Pipeline execution flow
- Response parsing and dataset conversion
- Error scenarios

---

## Minor Suggestions

1. **Documentation**: Add module-level docs to `src/adapters/dimse/mod.rs` explaining the adapter's role.

2. **Telemetry**: Consider adding OpenTelemetry spans around key operations (association handling, query execution).

3. **Configuration Validation**: `DimseConfig::validate()` creates storage directories if missing - this side effect might be surprising. Consider making it explicit or documenting.

4. **Command Codes**: Magic numbers like `0x0030` (C-ECHO-RQ) in `crates/dimse/src/scp/commands/mod.rs:28` could be named constants.

---

## Test Status

All existing tests pass:
- `cargo test --package dimse`: 38 passed, 1 ignored
- `cargo test dimse` (main crate): 27 passed

---

## Summary

The DIMSE subsystem rebuild is solid with good separation between the dimse crate and Harmony integration. The main concern is the **JSON-to-DatasetStream conversion** that may produce invalid DICOM data when the response flow expects actual DICOM bytes. The remaining items are cleanup and robustness improvements.

### Action Items

| Priority | Item | Effort |
|----------|------|--------|
| HIGH | Fix DatasetStream JSON conversion | Medium |
| MEDIUM | Expand VR mapping | Low |
| MEDIUM | Handle DCMTK shutdown properly | Low |
| MEDIUM | Extract tag constants | Low |
| LOW | Remove dead code | Trivial |
| LOW | Use async fs operations | Low |
| LOW | Add integration tests | Medium |
