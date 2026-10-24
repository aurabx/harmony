---
title: Using Envelopes
description: How middleware and backends read and change envelopes, with protocol-specific examples.
status: active
updated: 2026-10-24
type: Architecture
related:
  - architecture/envelope.md
---

# Using Envelopes

How middleware and backends read and change envelopes, with protocol-specific examples. This page is part of [Envelope: Data Exchange Format](envelope.md).

## Usage in Middleware

### Reading Data

```rust
impl Middleware for MyMiddleware {
    async fn process_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
    ) -> Result<RequestEnvelope<Vec<u8>>, MiddlewareError> {
        // Access normalized JSON
        if let Some(json) = &envelope.normalized_data {
            let name = json["name"].as_str();
        }
        
        // Access metadata
        let method = &envelope.request_details.method;
        
        Ok(envelope)
    }
}
```

### Transforming Data

```rust
impl Middleware for TransformMiddleware {
    async fn process_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
    ) -> Result<RequestEnvelope<Vec<u8>>, MiddlewareError> {
        // Transform normalized data
        if let Some(json) = envelope.normalized_data.as_mut() {
            json["transformed"] = Value::Bool(true);
        }
        
        // Update original_data from transformed JSON
        envelope.original_data = serde_json::to_vec(
            &envelope.normalized_data
        )?;
        
        Ok(envelope)
    }
}
```

## Usage in Backends

```rust
impl Backend for HttpBackend {
    async fn send(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, BackendError> {
        // Convert envelope to HTTP request
        let request = reqwest::Client::new()
            .post(&self.target_url)
            .body(envelope.original_data)
            .headers(envelope.request_details.headers);
        
        // Send request
        let response = request.send().await?;
        
        // Convert HTTP response to ResponseEnvelope
        Ok(ResponseEnvelope {
            response_details: ResponseDetails {
                status: response.status().as_u16(),
                headers: extract_headers(&response),
                metadata: HashMap::new(),
            },
            original_data: response.bytes().await?.to_vec(),
            normalized_data: None,
        })
    }
}
```

## Command Resolution

The `RequestEnvelope` provides two helper methods for reading the operational intent with automatic fallback to legacy metadata:

### `resolve_operation()`

Returns `Option<Operation>`. Reads `envelope.command` and returns the operation if set, or `None` if no command is present. Available on both `RequestEnvelope` and `ResponseEnvelope`.

```rust
let operation = envelope.resolve_operation();
match operation {
    Some(Operation::Query) => { /* C-FIND, search, QIDO-RS */ }
    Some(Operation::Retrieve) => { /* C-GET, read, WADO-RS */ }
    Some(Operation::Store) => { /* C-STORE, create, STOW-RS */ }
    Some(Operation::Ping) => { /* C-ECHO, health check */ }
    None => { /* No operation set -- use existing logic */ }
    _ => { /* Other operations */ }
}
```

### `resolve_command()`

Returns `Option<Command>` (the full Command with params and source). Same fallback chain as `resolve_operation()`, but constructs a `Command` from legacy metadata if needed.

For details on the Command system, available Operations, and how endpoints and backends use them, see [commands.md](commands.md).

## Best Practices

### 1. Preserve Original Data

Always keep `original_data` intact for protocol fidelity:

```rust
// ✅ Good: Transform normalized_data, sync to original_data
envelope.normalized_data = transform(envelope.normalized_data)?;
envelope.original_data = serialize(envelope.normalized_data)?;

// ❌ Bad: Lose original_data
envelope.original_data = Vec::new();
```

### 2. Use Metadata for Routing

Store routing decisions in metadata, not payload:

```rust
envelope.request_details.metadata.insert(
    "backend_target".to_string(),
    "pacs_server_1".to_string(),
);
```

### 3. Leverage Normalized Snapshot

Use `normalized_snapshot` for debugging transforms:

```rust
if let Some(before) = &envelope.normalized_snapshot {
    if let Some(after) = &envelope.normalized_data {
        tracing::debug!(
            "Transform changed {} fields",
            count_changes(before, after)
        );
    }
}
```

### 4. Error Context

Include envelope metadata in errors:

```rust
MiddlewareError::TransformFailed {
    reason: "Invalid JOLT spec".into(),
    request_id: envelope.request_details.metadata
        .get("request_id")
        .cloned(),
}
```

## Protocol-Specific Examples

### HTTP → DICOM Backend

```
HTTP POST /dicom/find
{
  "PatientID": "12345",
  "QueryLevel": "STUDY"
}
  ↓ HttpAdapter
RequestEnvelope {
  request_details: { method: "POST", uri: "/dicom/find", ... },
  command: Command { operation: Store },  // From HTTP POST
  original_data: b'{"PatientID": "12345", ...}',
  normalized_data: { "PatientID": "12345", ... },
}
  ↓ Metadata Transform Middleware
RequestEnvelope {
  ...,
  command: Command { operation: Query },   // Refined by middleware
}
  ↓ DICOM Backend (reads resolve_operation() → Query → C-FIND)
C-FIND request to PACS
  ↓
ResponseEnvelope {
  response_details: { status: 200, ... },
  original_data: <DICOM datasets>,
  normalized_data: [{ "StudyInstanceUID": "1.2.3...", ... }],
}
  ↓ HttpAdapter
HTTP 200 OK
[{ "StudyInstanceUID": "1.2.3...", ... }]
```

### DIMSE → HTTP Backend

```
C-FIND request from remote AET
  ↓ DimseAdapter
RequestEnvelope {
  request_details: { method: "C-FIND", ... },
  command: Command { operation: Query },     // Set by DICOM SCP endpoint
  metadata: { "calling_aet": "REMOTE" },
  original_data: <DICOM dataset>,
  normalized_data: { "PatientID": "12345", ... },
}
  ↓ HTTP Backend (reads resolve_operation() → Query → GET)
GET https://fhir.example.com/Patient?identifier=12345
  ↓
ResponseEnvelope {
  response_details: { status: 200, ... },
  original_data: <FHIR JSON>,
}
  ↓ DimseAdapter
C-FIND response with DICOM datasets
```
