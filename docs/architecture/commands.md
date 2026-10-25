---
title: Commands - Protocol-Agnostic Operations
description: The Command type that lets endpoints and backends exchange protocol-agnostic operations.
status: active
updated: 2026-10-24
type: Architecture
---

# Commands: Protocol-Agnostic Operations

## Overview

The Command system provides a **typed, protocol-agnostic way to express operational intent** on an Envelope. Instead of relying on implicit conventions like metadata strings or HTTP method inference, requests and responses flowing through the pipeline carry an explicit `Command` that says what should happen -- regardless of the source or destination protocol.

Endpoints set commands. Backends interpret them. Middleware can read or modify them.

> **For extension developers**: If you are building custom services via the extension system, see [Extension Types](extensions-types.md#service-factories) for how to register your service, then return here for how to produce and consume Commands.

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

## Producing and Consuming Commands

How endpoints produce commands, how backends consume them, and the command parameter and builder APIs. See [Producing and Consuming Commands](commands-producers-consumers.md).

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
