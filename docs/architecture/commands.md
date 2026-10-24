---
title: Commands: Protocol-Agnostic Operations
description: The Command type that lets endpoints and backends exchange protocol-agnostic operations.
status: active
updated: 2026-02-08
type: Architecture
---

# Commands: Protocol-Agnostic Operations

## Overview

The Command system provides a **typed, protocol-agnostic way to express operational intent** on an Envelope. Instead of relying on implicit conventions like metadata strings or HTTP method inference, requests and responses flowing through the pipeline carry an explicit `Command` that says what should happen -- regardless of the source or destination protocol.

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

### Envelope Integration

The `command` field is `Option<Command>` on both `RequestEnvelope` and `ResponseEnvelope`:

```rust
pub struct RequestEnvelope<T> {
    pub command: Option<Command>,       // <-- the Command
    // ...other fields...
}

pub struct ResponseEnvelope<T> {
    pub command: Option<Command>,       // <-- carried through from request
    // ...other fields...
}
```

Being `Option` means envelopes without a command continue to work. Backends use `resolve_operation()` which returns `None` when no command is set, allowing fallback to path-based or method-based detection.

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
ResponseEnvelope (carries command from request)
  |
  v
Middleware chain (right)
  Can read envelope.resolve_operation() on the response
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

### Protocol-Specific Endpoints

Protocol-aware endpoints (DICOMweb, FHIR, DICOM SCP, JMIX) set commands based on their protocol semantics:

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

### HTTP Endpoint (Generic Ingress)

The `HttpEndpoint` does **not** set a Command on inbound requests. It is a generic ingress that doesn't know the protocol semantics of the downstream backend. Protocol-aware middleware or backends resolve the operation from the path, metadata, or their own endpoint logic.

When acting as a **backend** (outbound), the HTTP endpoint reads `envelope.command` to override the HTTP method for protocol bridging (e.g., a DICOM C-FIND → `Operation::Query` → `GET` request).

### DIMSE Operation Mapping

For DICOM SCP and similar endpoints that receive DIMSE operations, create commands directly:

| DIMSE operation | Operation |
|-----------------|-----------|
| `find`          | `Query`   |
| `get`           | `Retrieve`|
| `move`          | `Move`    |
| `store`         | `Store`   |
| `echo`          | `Ping`    |

```rust
let command = match dimse_op.as_str() {
    "find" => Some(Command::new(Operation::Query)),
    "get" => Some(Command::new(Operation::Retrieve)),
    "move" => Some(Command::new(Operation::Move)),
    "store" => Some(Command::new(Operation::Store)),
    "echo" => Some(Command::new(Operation::Ping)),
    _ => None,
};
```

`Operation::dimse_op()` gives the reverse mapping. It returns `None` for operations with no DIMSE service.

The operation is not stored in envelope metadata. Between two Harmony instances, the `dicom_to_http` middleware carries it in the `x-dimse-op` HTTP header. The DICOM backends read the `Command` first and that header second.

## Consuming Commands (Backends)

Backends are **consumers** -- they read the command from the envelope to decide what protocol action to perform.

### Using `resolve_operation()`

Use `resolve_operation()` to get the operation. It returns `None` when no command is set, allowing backends to fall through to other resolution strategies:

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

`resolve_operation()` checks `envelope.command` and returns the operation if set. If no command is present, returns `None`. Backends should implement their own fallback logic (path-based detection, HTTP method mapping, etc.) in the `None` branch.

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

## Response Commands

`ResponseEnvelope` carries an `Option<Command>` field that is propagated from the request through the pipeline. This allows middleware `right()` methods to access the operation directly:

```rust
async fn right(
    &self,
    envelope: ResponseEnvelope<Value>,
) -> Result<ResponseEnvelope<Value>, Error> {
    match envelope.resolve_operation() {
        Some(Operation::Query) => { /* format query results */ }
        Some(Operation::Retrieve) => { /* format retrieval results */ }
        _ => {}
    }
    Ok(envelope)
}
```

Use `ResponseEnvelope::from_backend_with_command()` to construct responses that carry the command forward:

```rust
let response = ResponseEnvelope::from_backend_with_command(
    envelope.request_details.clone(),
    status,
    headers,
    body,
    normalized,
    envelope.command.clone(),  // propagate command
);
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

Protocol-bridging middleware can set the command when translating between protocols:

```rust
use crate::models::envelope::command::{Command, Operation};

// Set the typed Command
envelope.command = Some(
    Command::new(Operation::Query)
        .with_source("my_bridge_middleware")
);
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

## File Locations

| File | Purpose |
|------|---------|
| `src/models/envelope/command.rs` | `Command` and `Operation` type definitions |
| `src/models/envelope/envelope.rs` | `command` field, builder methods, `resolve_operation()` on both Request and Response envelopes |
| `src/models/envelope/mod.rs` | Module exports |

## Current Implementation Status

All endpoints, backends, and protocol-bridging middleware now produce and consume Commands:

| Component | Role | Status |
|-----------|------|--------|
| `DicomScpEndpoint` | Producer | Sets command from DIMSE operation |
| `HttpEndpoint` | Ingress | Does **not** set command (generic ingress) |
| `HttpEndpoint` (backend) | Consumer | Reads command to override HTTP method |
| `DicomwebEndpoint` | Producer | Maps DICOMweb routes (QIDO/WADO/STOW) |
| `FhirEndpoint` | Producer | Maps FHIR interactions (search/read/create/etc.) |
| `EchoEndpoint` | Producer | Always `Ping` |
| `JmixEndpoint` | Producer | Maps JMIX routes (retrieve/query/store) |
| `MockDicomEndpoint` | Producer | Sets command from DIMSE operation |
| `DicomScuBackend` | Consumer | Uses `resolve_operation()` with path fallback |
| `Http3Backend` | Consumer | Command overrides HTTP method |
| `JmixBackend` | Consumer | Command overrides HTTP method |
| `StorageBackend` | Consumer | Maps Operation to read/write |
| `MockDicomEndpoint` (backend) | Consumer | Uses `resolve_operation()` |
| `DICOMwebToDICOM` middleware | Bridge | Sets command for DICOM backends |
| `DicomToDicomweb` middleware | Bridge | Reads via `resolve_operation()` on response |
| `JoltTransform` middleware | Plumbing | Passes command through context |
| `MetadataTransform` middleware | Plumbing | Passes command through context |

## See Also

- [envelope.md](envelope.md) - Core Envelope data exchange format
- [endpoints.md](../configuration/endpoints.md) - Endpoint types and configuration
- [backends.md](../configuration/backends.md) - Backend types and target communication
- [middleware.md](../configuration/middleware.md) - Middleware reference
- [extensions.md](extensions.md) - Extension system for custom services
- [adapters.md](adapters.md) - Protocol adapter architecture
