# Envelope: Data Exchange Format

**Last Updated**: 2025-01-18 (Phase 6)

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

## RequestEnvelope

The `RequestEnvelope` wraps incoming request data in a protocol-agnostic format.

### Structure

```rust
pub struct RequestEnvelope<T> {
    pub request_details: RequestDetails,
    pub original_data: T,
    pub normalized_data: Option<serde_json::Value>,
    pub normalized_snapshot: Option<serde_json::Value>,
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `request_details` | `RequestDetails` | Metadata about the request (method, URI, headers, etc.) |
| `original_data` | `T` (generic) | Raw protocol payload (bytes, JSON, DICOM dataset, etc.) |
| `normalized_data` | `Option<serde_json::Value>` | JSON representation of `original_data` |
| `normalized_snapshot` | `Option<serde_json::Value>` | Pre-transform snapshot for debugging |

### RequestDetails

Contains protocol-agnostic request metadata:

```rust
pub struct RequestDetails {
    pub method: String,              // HTTP method, DIMSE operation, etc.
    pub uri: String,                 // Request path or identifier
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub query_params: HashMap<String, Vec<String>>,
    pub cache_status: Option<String>,
    pub metadata: HashMap<String, String>,  // Protocol-specific metadata
}
```

**Common metadata keys**:
- `dimse_op`: DICOM operation (e.g., `C-FIND`, `C-STORE`)
- `request_id`: Unique request identifier
- `protocol`: Source protocol (e.g., `http`, `dimse`, `hl7`)

## ResponseEnvelope

The `ResponseEnvelope` wraps response data returned from the pipeline.

### Structure

```rust
pub struct ResponseEnvelope<T> {
    pub response_details: ResponseDetails,
    pub original_data: T,
    pub normalized_data: Option<serde_json::Value>,
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `response_details` | `ResponseDetails` | Status code, headers, metadata |
| `original_data` | `T` (generic) | Response payload |
| `normalized_data` | `Option<serde_json::Value>` | JSON representation of response |

### ResponseDetails

```rust
pub struct ResponseDetails {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub metadata: HashMap<String, String>,
}
```

## ProtocolCtx

Provides protocol-specific context throughout the pipeline.

### Structure

```rust
pub struct ProtocolCtx {
    pub protocol: Protocol,           // HTTP, DIMSE, HL7, etc.
    pub payload: Vec<u8>,            // Raw protocol payload
    pub meta: HashMap<String, String>,
    pub attrs: serde_json::Value,    // Protocol-specific attributes
}
```

### Protocol Enum

```rust
pub enum Protocol {
    Http,
    Dimse,
    Hl7,
    Custom(String),
}
```

## Pipeline Flow

### Request Flow

```
1. Protocol Adapter receives protocol request
   ↓
2. Service builds RequestEnvelope from ProtocolCtx
   ├─ Extracts RequestDetails (method, URI, headers)
   ├─ Stores raw payload in original_data
   └─ Normalizes to JSON in normalized_data
   ↓
3. PipelineExecutor processes RequestEnvelope
   ├─ Incoming middleware (auth, transforms)
   ├─ Backend invocation
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
  original_data: b'{"PatientID": "12345", ...}',
  normalized_data: { "PatientID": "12345", ... },
}
  ↓ Metadata Transform Middleware
RequestEnvelope {
  ...,
  metadata: { "dimse_op": "C-FIND" },  // Added by middleware
}
  ↓ DICOM Backend
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
  metadata: { "dimse_op": "C-FIND", "calling_aet": "REMOTE" },
  original_data: <DICOM dataset>,
  normalized_data: { "PatientID": "12345", ... },
}
  ↓ HTTP Backend
GET https://fhir.example.com/Patient?identifier=12345
  ↓
ResponseEnvelope {
  response_details: { status: 200, ... },
  original_data: <FHIR JSON>,
}
  ↓ DimseAdapter
C-FIND response with DICOM datasets
```

## See Also

- [adapters.md](adapters.md) - Protocol adapter architecture
- [router.md](router.md) - Pipeline execution flow
- [middleware.md](middleware.md) - Middleware usage
- [backends.md](backends.md) - Backend implementations
