---
title: Envelope Types
description: Field reference for RequestEnvelope, ResponseEnvelope, and ProtocolCtx.
status: active
updated: 2026-10-24
type: Architecture
related:
  - architecture/envelope.md
---

# Envelope Types

Field reference for RequestEnvelope, ResponseEnvelope, and ProtocolCtx. This page is part of [Envelope: Data Exchange Format](envelope.md).

## RequestEnvelope

The `RequestEnvelope` wraps incoming request data in a protocol-agnostic format.

### Structure

```rust
pub struct RequestEnvelope<T> {
    pub request_details: RequestDetails,
    pub backend_request_details: RequestDetails,
    pub target_details: Option<TargetDetails>,
    pub command: Option<Command>,
    pub original_data: T,
    pub normalized_data: Option<serde_json::Value>,
    pub normalized_snapshot: Option<serde_json::Value>,
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `request_details` | `RequestDetails` | Metadata about the incoming request (method, URI, headers, etc.) |
| `backend_request_details` | `RequestDetails` | Request details for the backend call (may differ from incoming request) |
| `target_details` | `Option<TargetDetails>` | Backend target connection info (base_url, method, headers, metadata). Set by the pipeline before backend invocation. |
| `command` | `Option<Command>` | The intended operation for the backend, set by the endpoint or middleware. See [commands.md](commands.md). |
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
    pub content_metadata: Option<ContentMetadata>,
}
```

### TargetDetails

Contains backend target connection information, populated by the pipeline before backend invocation:

```rust
pub struct TargetDetails {
    pub base_url: String,
    pub method: String,
    pub uri: String,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub query_params: HashMap<String, Vec<String>>,
    pub metadata: HashMap<String, String>,
}
```

**Common metadata keys**:
- `path`: Request subpath without leading slash (e.g., `"Patient/123"`)
- `path_with_query`: Subpath with query string (e.g., `"Patient/123?_count=5"`)
- `full_path`: Complete path with prefix and leading slash (e.g., `"/fhir/Patient/123"`)
- `dicom_operation`: DICOM operation string (e.g., `"find"`, `"store"`). Set by some adapters for informational purposes. Prefer using the typed `Command` on the envelope. See [commands.md](commands.md).
- `request_id`: Unique request identifier
- `protocol`: Source protocol (e.g., `http`, `dimse`, `hl7`)

### Path Handling

**Critical:** The `path` metadata field stores paths **without leading slashes**. Always use `path_utils::extract_path()` when constructing backend URLs.

```rust
use crate::models::services::path_utils::extract_path;

// ✅ CORRECT: Use extract_path() for URL construction
let path = extract_path(&envelope);  // Returns: "/Patient/123" (with slash)
let url = format!("{}{}", base_url, path);

// ❌ WRONG: Direct metadata access
let path = envelope.request_details.metadata.get("path").unwrap();
let url = format!("{}{}", base_url, path);  // Broken! Missing slash
```

**Path metadata fields:**

| Field | Format | Example | Purpose |
|-------|--------|---------|----------|
| `path` | No leading slash | `"Patient/123"` | Normalized subpath for comparisons |
| `path_with_query` | With leading slash | `"/Patient/123?_count=5"` | Full path with query params |
| `full_path` | With leading slash | `"/fhir/Patient/123"` | Complete path including prefix |

**Why no leading slash in `path`?**
- Consistency: Always normalized for string matching
- Safety: The `extract_path()` helper adds the slash when needed
- Clarity: Explicit about whether a path is a subpath or full path

**When to use each:**
- `extract_path()`: Backend URL construction (always use this!)
- `metadata["path"]`: String matching, routing logic (already normalized)
- `metadata["full_path"]`: Logging, debugging (shows complete request path)

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
