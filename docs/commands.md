# Commands: Protocol-Agnostic Operations

**Last Updated**: 2026-02-08

## Overview

The Command system provides a **typed, protocol-agnostic way to express operational intent** on a `RequestEnvelope`. Instead of relying on implicit conventions like metadata strings (`metadata["dimse_op"]`) or HTTP method inference, every request flowing through the pipeline carries an explicit `Command` that says what should happen -- regardless of the source or destination protocol.

Endpoints set commands. Backends interpret them. Middleware can read or modify them.

> **For extension developers**: If you are building custom services via the extension system, see [extensions.md](extensions.md#service-factories) for how to register your service, then return here for how to produce and consume Commands.

## Key Types

### Command

A `Command` expresses the intended operation in universal terms:

```rust
pub struct Command {
    pub operation: Operation,
    pub params: HashMap<String, serde_json::Value>,
    pub source: Option<String>,
}
```

| Field | Type | Description |
|-------|------|-------------|
| `operation` | `Operation` | The universal operation to perform |
| `params` | `HashMap<String, Value>` | Operation-specific parameters (flexible key-value) |
| `source` | `Option<String>` | Debugging: who set this command (e.g., `"dicom_scp_endpoint"`) |

### Operation

The `Operation` enum defines the universal vocabulary of operations:

| Operation   | DICOM SCU    | DICOMweb   | HTTP        | FHIR     | Storage   |
|-------------|--------------|------------|-------------|----------|-----------|
| `Query`     | C-FIND       | QIDO-RS    | GET (list)  | search   | read      |
| `Retrieve`  | C-GET/C-MOVE | WADO-RS    | GET (item)  | read     | read      |
| `Store`     | C-STORE      | STOW-RS    | POST        | create   | write     |
| `Update`    | -            | -          | PUT         | update   | write     |
| `Patch`     | -            | -          | PATCH       | patch    | -         |
| `Delete`    | -            | DELETE     | DELETE      | delete   | -         |
| `Ping`      | C-ECHO       | -          | HEAD/OPTIONS| metadata | -         |
| `Subscribe` | -            | -          | -           | Subscription | -    |
| `Publish`   | -            | -          | -           | -        | -         |
| `Custom`    | (action key) | (action key)| (action key)| (action key) | - |

The key design principle: endpoints set an **operation** (what they want done), backends decide **how** to do it in their protocol.

### RequestEnvelope Integration

The `command` field is `Option<Command>` on `RequestEnvelope`:

```rust
pub struct RequestEnvelope<T> {
    pub request_details: RequestDetails,
    pub backend_request_details: RequestDetails,
    pub target_details: Option<TargetDetails>,
    pub command: Option<Command>,       // <-- the Command
    pub original_data: T,
    pub normalized_data: Option<serde_json::Value>,
    pub normalized_snapshot: Option<serde_json::Value>,
}
```

Being `Option` means existing envelopes without a command continue to work. The `resolve_operation()` method provides automatic fallback to legacy metadata.

## How It Works

### Pipeline Flow

```
Protocol Request
  |
  v
Endpoint (producer)
  Sets envelope.command based on incoming request
  |
  v
Middleware chain (left)
  Can read, modify, or set envelope.command
  Protocol-bridge middleware translates between protocols
  |
  v
Backend (consumer)
  Reads envelope.resolve_operation() to decide what to do
  Maps Operation to protocol-specific action
  |
  v
Middleware chain (right)
  Response processing (command not on ResponseEnvelope)
  |
  v
Protocol Response
```

### Example: DICOMweb QIDO-RS Query

```
GET /studies?PatientID=12345
  |
  v
DicomwebEndpoint.endpoint_incoming_request()
  Classifies route: GET /studies -> QIDO-RS
  Sets: envelope.command = Command(Operation::Query)
  |
  v
DICOMwebToDICOM middleware.left()
  Reads: envelope.command.operation == Query
  Sets: metadata["dimse_op"] = "find" (backward compat)
  Builds DICOM identifier dataset
  |
  v
DicomScuBackend.backend_outgoing_request()
  Reads: envelope.resolve_operation() -> Some(Operation::Query)
  Maps: Query -> C-FIND
  Sends C-FIND to PACS
```

## Producing Commands (Endpoints)

Endpoints are **producers** -- they determine the operation from the incoming request and set the command on the envelope.

### In `build_protocol_envelope()`

For endpoints that build envelopes directly (DICOM SCP, MockDicom):

```rust
use crate::models::envelope::command::Command;

async fn build_protocol_envelope(
    &self,
    ctx: ProtocolCtx,
    _options: &HashMap<String, Value>,
) -> Result<RequestEnvelope<Vec<u8>>, Error> {
    let op = ctx.meta.get("dimse_op").cloned().unwrap_or_default();

    // Map the protocol operation to a typed Command
    let command = Command::from_legacy_dimse_op(&op)
        .map(|c| c.with_source("my_endpoint"));

    RequestEnvelope::builder()
        .method(op)
        .uri(uri)
        .command(command)   // <-- set the command
        .original_data(ctx.payload)
        .build()
}
```

### In `endpoint_incoming_request()`

For endpoints that delegate `build_protocol_envelope` to `HttpEndpoint` and then refine the command (DICOMweb, FHIR, JMIX, Echo):

```rust
use crate::models::envelope::command::{Command, Operation};

async fn endpoint_incoming_request(
    &self,
    mut envelope: RequestEnvelope<Vec<u8>>,
    _options: &HashMap<String, Value>,
) -> Result<RequestEnvelope<Vec<u8>>, Error> {
    let method = envelope.request_details.method.to_uppercase();
    let subpath = envelope.request_details.metadata
        .get("path").cloned().unwrap_or_default();

    // Classify the operation from route + method
    let operation = match (method.as_str(), subpath.as_str()) {
        ("GET", "studies") => Some(Operation::Query),       // QIDO-RS
        ("GET", _) => Some(Operation::Retrieve),            // WADO-RS
        ("POST", "studies") => Some(Operation::Store),      // STOW-RS
        _ => None,
    };

    if let Some(op) = operation {
        envelope.command = Some(Command::new(op).with_source("my_endpoint"));
    }

    Ok(envelope)
}
```

### HTTP Method Mapping

The `HttpEndpoint` sets a default Command from the HTTP method. Protocol-specific endpoints (DICOMweb, FHIR, JMIX) then **override** this with a more precise Operation in `endpoint_incoming_request()`.

| HTTP Method | Default Operation | May Be Refined To |
|-------------|-------------------|-------------------|
| `GET`       | `Retrieve`        | `Query` (for list/search endpoints) |
| `POST`      | `Store`           | `Query` (for FHIR `_search`) |
| `PUT`       | `Update`          | - |
| `PATCH`     | `Patch`           | - |
| `DELETE`    | `Delete`          | - |
| `HEAD`      | `Ping`            | - |
| `OPTIONS`   | `Ping`            | - |

### DIMSE Operation Mapping

Use `Command::from_legacy_dimse_op()` for backward-compatible mapping from DIMSE operation strings:

| dimse_op string | Operation |
|-----------------|-----------|
| `"find"`        | `Query`   |
| `"get"`         | `Retrieve`|
| `"move"`        | `Retrieve`|
| `"store"`       | `Store`   |
| `"echo"`        | `Ping`    |

The `from_legacy_dimse_op()` method is case-insensitive and sets `source: "legacy_dimse_op"` for debugging.

## Consuming Commands (Backends)

Backends are **consumers** -- they read the command from the envelope to decide what protocol action to perform.

### Using `resolve_operation()`

Always use `resolve_operation()` instead of reading `envelope.command` directly. This method provides automatic fallback to legacy `metadata["dimse_op"]` if no Command is set:

```rust
use crate::models::envelope::command::Operation;

async fn backend_outgoing_request(
    &self,
    envelope: RequestEnvelope<Vec<u8>>,
    options: &HashMap<String, Value>,
) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
    let operation = envelope.resolve_operation();

    match operation {
        Some(Operation::Query) => { /* C-FIND, QIDO-RS, search, etc. */ }
        Some(Operation::Retrieve) => { /* C-GET, WADO-RS, read, etc. */ }
        Some(Operation::Store) => { /* C-STORE, STOW-RS, create, etc. */ }
        Some(Operation::Update) => { /* PUT, update */ }
        Some(Operation::Delete) => { /* DELETE */ }
        Some(Operation::Ping) => { /* C-ECHO, HEAD */ }
        Some(op) => {
            return Err(Error::from(format!(
                "Operation '{}' not supported by this backend", op
            )));
        }
        None => {
            // No command set -- fall back to existing behavior
            // (e.g., use HTTP method, path-based detection, etc.)
        }
    }
}
```

### Resolution Order

`resolve_operation()` checks in this order:

1. **`envelope.command`** -- explicit Command (preferred)
2. **`target_details.metadata["dimse_op"]`** -- legacy metadata on target
3. **`request_details.metadata["dimse_op"]`** -- legacy metadata on request

If none are found, returns `None`.

### HTTP Backend Pattern

For HTTP-based backends (HTTP, HTTP/3, JMIX), the Command can **override** the HTTP method on `target_details`:

```rust
if let Some(ref cmd) = envelope.command {
    let overridden_method = match cmd.operation {
        Operation::Query | Operation::Retrieve => "GET",
        Operation::Store => "POST",
        Operation::Update => "PUT",
        Operation::Patch => "PATCH",
        Operation::Delete => "DELETE",
        Operation::Ping => "HEAD",
        _ => target_details.method.as_str(),
    };
    if overridden_method != target_details.method {
        target_details.method = overridden_method.to_string();
    }
}
```

This enables protocol-bridging scenarios where a DICOM C-FIND arrives, gets translated to `Operation::Query`, and the HTTP backend maps that to a `GET` request.

### Storage Backend Pattern

For storage backends, map Operations to read/write:

```rust
let is_read = match envelope.resolve_operation() {
    Some(Operation::Query) | Some(Operation::Retrieve) => true,
    Some(Operation::Store) | Some(Operation::Update) => false,
    _ => method == "GET",  // fallback to HTTP method
};
```

## Middleware and Commands

### Reading Commands

Middleware can inspect the command to make routing decisions:

```rust
async fn left(
    &self,
    envelope: RequestEnvelope<Value>,
) -> Result<RequestEnvelope<Value>, Error> {
    if let Some(ref cmd) = envelope.command {
        match cmd.operation {
            Operation::Query => { /* prepare query-specific transforms */ }
            Operation::Store => { /* validate payload for storage */ }
            _ => {}
        }
    }
    Ok(envelope)
}
```

### Setting Commands (Protocol Bridges)

Protocol-bridging middleware can set the command when translating between protocols. When doing so, **also set the legacy metadata** for backward compatibility:

```rust
// Set the typed Command (modern path)
use crate::models::envelope::command::Command;
if let Some(cmd) = Command::from_legacy_dimse_op(op_name) {
    envelope.command = Some(cmd.with_source("my_bridge_middleware"));
}

// Also set legacy metadata (backward compatibility)
envelope.request_details.metadata
    .insert("dimse_op".to_string(), op_name.to_string());
```

### Transform Middleware Integration

The built-in `JoltTransformMiddleware` and `MetadataTransformMiddleware` automatically include `command` in the JSON context passed to transforms. If the transform output includes a `command` field, it is deserialized back as a `Command`:

**Transform input context:**
```json
{
  "data": { ... },
  "context": {
    "request_details": { ... },
    "target_details": { ... },
    "command": {
      "operation": "query",
      "params": { "query_level": "STUDY" }
    }
  }
}
```

**Transform output (can modify command):**
```json
{
  "data": { ... },
  "context": {
    "command": {
      "operation": "retrieve",
      "params": { "retrieve_mode": "get" }
    }
  }
}
```

## Command Parameters

The `params` field provides flexible key-value parameters for operation-specific data. Backends ignore unknown parameters.

### Common Parameters

| Operation   | Param Key        | Type   | Description |
|-------------|------------------|--------|-------------|
| `Query`     | `query_level`    | String | DICOM query level: `"PATIENT"`, `"STUDY"`, `"SERIES"`, `"IMAGE"` |
| `Query`     | `limit`          | Number | Maximum results to return |
| `Query`     | `offset`         | Number | Pagination offset |
| `Retrieve`  | `retrieve_mode`  | String | DICOM retrieval mode: `"get"` (C-GET) or `"move"` (C-MOVE) |
| `Retrieve`  | `format`         | String | Desired response format |
| `Store`     | `overwrite`      | Bool   | Allow overwriting existing resources |
| `Store`     | `validate`       | Bool   | Validate payload before storing |
| `Custom`    | `action`         | String | **(required)** The specific custom action name |

### Setting Parameters

```rust
let cmd = Command::new(Operation::Retrieve)
    .with_param("retrieve_mode", serde_json::json!("get"))
    .with_source("dicomweb_to_dicom_middleware");

envelope.command = Some(cmd);
```

### Reading Parameters

```rust
if let Some(ref cmd) = envelope.command {
    let retrieve_mode = cmd.params.get("retrieve_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("get");
}
```

## Builder API

The `RequestEnvelopeBuilder` supports setting commands in two ways:

```rust
// Option 1: Set a full Command object
let cmd = Command::new(Operation::Query)
    .with_param("limit", serde_json::json!(100))
    .with_source("my_endpoint");

RequestEnvelope::builder()
    .method("GET")
    .uri("/studies")
    .command(Some(cmd))
    .original_data(payload)
    .build()

// Option 2: Shorthand -- set just an Operation (creates a basic Command)
RequestEnvelope::builder()
    .method("GET")
    .uri("/studies")
    .operation(Operation::Query)
    .original_data(payload)
    .build()
```

## Backward Compatibility

The Command system is fully backward compatible:

1. **`command` is `Option<Command>`** -- envelopes without a command continue to work
2. **`resolve_operation()` falls back to legacy metadata** -- checks `metadata["dimse_op"]` if no command is set
3. **Backends handle `None`** -- the `None` arm falls through to existing method-based or path-based logic
4. **Dual-write in middleware** -- protocol-bridge middleware sets both `command` and `metadata["dimse_op"]` during the transition

### Migration Path for Existing Code

If your code reads `metadata["dimse_op"]` directly:

```rust
// Before (legacy)
let op = envelope.request_details.metadata.get("dimse_op")
    .cloned()
    .unwrap_or_default();

// After (modern, with automatic fallback)
let operation = envelope.resolve_operation();
match operation {
    Some(Operation::Query) => { /* was "find" */ }
    Some(Operation::Retrieve) => { /* was "get" or "move" */ }
    Some(Operation::Store) => { /* was "store" */ }
    Some(Operation::Ping) => { /* was "echo" */ }
    None => { /* no operation set */ }
    _ => { /* other operations */ }
}
```

## File Locations

| File | Purpose |
|------|---------|
| `src/models/envelope/command.rs` | `Command` and `Operation` type definitions |
| `src/models/envelope/envelope.rs` | `command` field, builder methods, `resolve_operation()` |
| `src/models/envelope/mod.rs` | Module exports |

## Current Implementation Status

All endpoints, backends, and protocol-bridging middleware now produce and consume Commands:

| Component | Role | Status |
|-----------|------|--------|
| `DicomScpEndpoint` | Producer | Sets command via `from_legacy_dimse_op()` |
| `HttpEndpoint` | Producer | Maps HTTP method to Operation |
| `DicomwebEndpoint` | Producer | Maps DICOMweb routes (QIDO/WADO/STOW) |
| `FhirEndpoint` | Producer | Maps FHIR interactions (search/read/create/etc.) |
| `EchoEndpoint` | Producer | Always `Ping` |
| `JmixEndpoint` | Producer | Maps JMIX routes (retrieve/query/store) |
| `MockDicomEndpoint` | Producer | Sets command via `from_legacy_dimse_op()` |
| `DicomScuBackend` | Consumer | Uses `resolve_operation()` with legacy fallback |
| `HttpEndpoint` (backend) | Consumer | Command overrides HTTP method |
| `Http3Backend` | Consumer | Command overrides HTTP method |
| `JmixBackend` | Consumer | Command overrides HTTP method |
| `StorageBackend` | Consumer | Maps Operation to read/write |
| `MockDicomEndpoint` (backend) | Consumer | Uses `resolve_operation()` |
| `DICOMwebToDICOM` middleware | Bridge | Sets command + legacy `dimse_op` |
| `DicomToDicomweb` middleware | Bridge | Reads via `resolve_operation()` |
| `JoltTransform` middleware | Plumbing | Passes command through context |
| `MetadataTransform` middleware | Plumbing | Passes command through context |

## See Also

- [envelope.md](envelope.md) - Core Envelope data exchange format
- [endpoints.md](endpoints.md) - Endpoint types and configuration
- [backends.md](backends.md) - Backend types and target communication
- [middleware.md](middleware.md) - Middleware reference
- [extensions.md](extensions.md) - Extension system for custom services
- [adapters.md](adapters.md) - Protocol adapter architecture
