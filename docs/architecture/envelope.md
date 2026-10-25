---
title: Envelope - Data Exchange Format
description: The RequestEnvelope and ResponseEnvelope types that carry data through every pipeline.
status: active
updated: 2026-10-24
type: Architecture
---

# Envelope: Data Exchange Format

## Overview

Envelopes are the core data exchange format in Harmony. They provide a protocol-agnostic way to pass data through the pipeline, enabling middleware, backends, and endpoints to work together regardless of the underlying protocol (HTTP, DIMSE, HL7, etc.).

## Architecture

```
Protocol Adapter → RequestEnvelope → Pipeline → ResponseEnvelope → Protocol Adapter
```

### Key Concepts

1. **RequestEnvelope**: Carries inbound request data through the pipeline
2. **ResponseEnvelope**: Carries response data back to the protocol adapter
3. **ProtocolCtx**: Protocol-specific context (method, headers, metadata)
4. **Normalization**: Protocol payloads are normalized to JSON for middleware processing

## Envelope Types

Field reference for RequestEnvelope, ResponseEnvelope, and ProtocolCtx. See [Envelope Types](envelope-types.md).

## Pipeline Flow

### Request Flow

```
1. Protocol Adapter receives protocol request
   ↓
2. Service builds RequestEnvelope from ProtocolCtx
   ├─ Extracts RequestDetails (method, URI, headers)
   ├─ Sets Command from protocol operation (e.g., HTTP method, DIMSE op)
   ├─ Stores raw payload in original_data
   └─ Normalizes to JSON in normalized_data
   ↓
3. PipelineExecutor processes RequestEnvelope
   ├─ Incoming middleware (auth, transforms, may modify Command)
   ├─ Backend invocation (reads Command via resolve_operation())
   └─ Outgoing middleware (response transforms)
   ↓
4. PipelineExecutor returns ResponseEnvelope
   ↓
5. Protocol Adapter converts to protocol response
```

### Example: HTTP Request

```rust
// 1. HTTP request arrives at HttpAdapter
let http_request = /* Axum Request */;

// 2. Service builds RequestEnvelope
let ctx = ProtocolCtx {
    protocol: Protocol::Http,
    payload: body_bytes,
    meta: extract_http_headers(&http_request),
    attrs: json!({ "method": "POST", "path": "/api/resource" }),
};

let envelope = service.build_protocol_envelope(ctx, &options).await?;

// 3. Execute through pipeline
let response = PipelineExecutor::execute(envelope, pipeline, config, &ctx).await?;

// 4. Convert to HTTP response
let http_response = Response::builder()
    .status(response.response_details.status)
    .body(response.original_data)
    .unwrap();
```

## Normalization

### Purpose

Normalization converts protocol-specific payloads to JSON, enabling:
- Protocol-agnostic middleware processing
- JOLT transformations
- Consistent logging and debugging

### When It Happens

1. **Request normalization**: When service builds `RequestEnvelope`
2. **Response normalization**: When backend returns `ResponseEnvelope`
3. **Snapshot**: Before transforms (preserves original for debugging)

### Example

```rust
// HTTP JSON request
original_data: b'{"name": "Alice", "age": 30}'
normalized_data: Value { "name": "Alice", "age": 30 }

// DICOM dataset
original_data: DicomDataset { ... }
normalized_data: Value { "PatientName": "Doe^John", ... }
```

## Using Envelopes

How middleware and backends read and change envelopes, with protocol-specific examples. See [Using Envelopes](envelope-usage.md).

## See Also

- [commands.md](commands.md) - Command system for protocol-agnostic operations
- [adapters.md](adapters.md) - Protocol adapter architecture
- [router.md](router.md) - Pipeline execution flow
- [middleware.md](../configuration/middleware.md) - Middleware usage
- [backends.md](../configuration/backends.md) - Backend implementations
