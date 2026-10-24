---
title: Producing and Consuming Commands
description: How endpoints produce commands, how backends consume them, and the command parameter and builder APIs.
status: active
updated: 2026-10-24
type: Architecture
related:
  - architecture/commands.md
---

# Producing and Consuming Commands

How endpoints produce commands, how backends consume them, and the command parameter and builder APIs. This page is part of [Commands: Protocol-Agnostic Operations](commands.md).

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
