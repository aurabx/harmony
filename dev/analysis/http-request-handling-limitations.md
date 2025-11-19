# HTTP Request Handling: Assumptions & Limitations

**Date**: 2025-01-12  
**Analysis**: Current behavior and missing capabilities  
**Impact**: Pipeline types requiring non-JSON content handling

---

## Executive Summary

The current HTTP endpoint implementation makes **strong assumptions about JSON** as the primary data format. This creates limitations for several pipeline types, particularly:

- ❌ **CSV to JSON/XML** (#1) - No CSV parsing
- ❌ **Form data to Database** (#3) - No form decoding
- ❌ **XML/SOAP to REST** (#10) - No XML parsing
- ⚠️ **Binary data handling** - Limited multipart support

**Root Cause**: The `HttpAdapter` treats the request body as **opaque bytes** and only attempts JSON normalization, with no content-type-aware parsing.

---

## Current Request Flow

### 1. HTTP Request → ProtocolCtx

**Location**: `src/adapters/http/mod.rs::http_request_to_protocol_ctx()`

```rust
// Lines 150-156
let body_bytes = axum::body::to_bytes(
    std::mem::replace(req.body_mut(), Body::empty()), 
    usize::MAX
).await?.to_vec();

Ok(ProtocolCtx {
    protocol: Protocol::Http,
    payload: body_bytes,  // ← Raw bytes, no parsing
    meta: meta_map,
    attrs: serde_json::Value::Object(attrs),
})
```

**What's captured:**
- ✅ Headers (all converted to string map)
- ✅ Cookies (parsed from Cookie header)
- ✅ Query params (URL-decoded)
- ✅ HTTP method
- ✅ URI/path
- ✅ Cache status
- ✅ Body bytes (raw, unparsed)

**What's missing:**
- ❌ Content-Type-aware body parsing
- ❌ Multipart form data handling
- ❌ URL-encoded form data parsing
- ❌ Character encoding detection
- ❌ Binary content type detection

---

### 2. ProtocolCtx → RequestEnvelope

**Location**: `src/models/services/types/http.rs::build_protocol_envelope()`

```rust
// Lines 158-159
let normalized_data = serde_json::from_slice(&ctx.payload).ok();

RequestEnvelope::builder()
    .method(method)
    .uri(uri)
    .headers(headers_map)
    .cookies(cookies_map)
    .query_params(query_params)
    .cache_status(cache_status)
    .metadata(metadata)
    .target_details(None)
    .original_data(ctx.payload)
    .normalized_data(normalized_data)  // ← Only attempts JSON parse
    .normalized_snapshot(None)
    .build()
```

**Assumptions:**
1. **JSON is the default format** - Always attempts `serde_json::from_slice()`
2. **Failures are silent** - `.ok()` discards parse errors
3. **No content-type inspection** - Doesn't check `Content-Type` header
4. **Binary data ignored** - No special handling for non-text content

**Result:**
- JSON requests: ✅ `normalized_data` populated correctly
- CSV requests: ❌ `normalized_data = None`, `original_data = bytes`
- XML requests: ❌ `normalized_data = None`, `original_data = bytes`
- Form data: ❌ `normalized_data = None`, `original_data = bytes`
- Binary files: ⚠️ `normalized_data = None`, `original_data = bytes` (correct, but no metadata)

---

### 3. Response Handling

**Location**: `src/models/services/types/http.rs::backend_outgoing_request()`

```rust
// Lines 317-329
if let Some(content_type) = response_envelope
    .response_details
    .headers
    .get("content-type")
{
    if content_type.contains("application/json") {
        if let Ok(json_value) = 
            serde_json::from_slice::<serde_json::Value>(&response_envelope.original_data)
        {
            response_envelope.normalized_data = Some(json_value);
        }
    }
}
```

**Behavior:**
- ✅ Checks `Content-Type` header for JSON
- ✅ Parses JSON responses automatically
- ❌ No XML response parsing
- ❌ No CSV response parsing
- ❌ No other format detection

**Better than request handling**, but still limited to JSON.

---

## Specific Request Type Analysis

### JSON Requests ✅

**Content-Type**: `application/json`, `application/fhir+json`, etc.

**Current Behavior**: Works correctly
- ✅ Parsed to `normalized_data`
- ✅ Available for JOLT transforms
- ✅ Original bytes preserved in `original_data`

**Example:**
```http
POST /api/resource HTTP/1.1
Content-Type: application/json

{"name": "Alice", "age": 30}
```

**Envelope:**
```rust
RequestEnvelope {
    original_data: b'{"name":"Alice","age":30}',
    normalized_data: Some(Value::Object({
        "name": String("Alice"),
        "age": Number(30)
    })),
}
```

---

### XML Requests ❌

**Content-Type**: `application/xml`, `text/xml`, `application/soap+xml`

**Current Behavior**: **NOT parsed**
- ❌ `normalized_data = None`
- ✅ `original_data` contains raw XML bytes
- ❌ Not available for middleware transforms
- ❌ Must be handled manually in custom middleware

**Example:**
```http
POST /api/resource HTTP/1.1
Content-Type: application/xml

<person>
  <name>Alice</name>
  <age>30</age>
</person>
```

**Envelope:**
```rust
RequestEnvelope {
    original_data: b'<person><name>Alice</name><age>30</age></person>',
    normalized_data: None,  // ← NOT PARSED
}
```

**Impact**: Blocks Pipeline #10 (SOAP→REST) without custom XML handler

---

### CSV Requests ❌

**Content-Type**: `text/csv`

**Current Behavior**: **NOT parsed**
- ❌ `normalized_data = None`
- ✅ `original_data` contains raw CSV bytes
- ❌ Not available for middleware transforms

**Example:**
```http
POST /upload/csv HTTP/1.1
Content-Type: text/csv

name,age
Alice,30
Bob,25
```

**Envelope:**
```rust
RequestEnvelope {
    original_data: b'name,age\nAlice,30\nBob,25',
    normalized_data: None,  // ← NOT PARSED
}
```

**Impact**: Blocks Pipeline #1 (CSV→JSON/XML) without custom CSV handler

---

### Form Data (URL-encoded) ❌

**Content-Type**: `application/x-www-form-urlencoded`

**Current Behavior**: **NOT parsed**
- ❌ Body not decoded into key-value pairs
- ❌ `normalized_data = None`
- ✅ `original_data` contains raw encoded bytes

**Example:**
```http
POST /form HTTP/1.1
Content-Type: application/x-www-form-urlencoded

name=Alice&age=30&interests%5B%5D=coding&interests%5B%5D=music
```

**Current Envelope:**
```rust
RequestEnvelope {
    original_data: b'name=Alice&age=30&interests%5B%5D=coding&interests%5B%5D=music',
    normalized_data: None,  // ← NOT PARSED
}
```

**Expected Envelope** (with parsing):
```rust
RequestEnvelope {
    original_data: b'name=Alice&age=30...',
    normalized_data: Some(Value::Object({
        "name": String("Alice"),
        "age": String("30"),
        "interests": Array([String("coding"), String("music")])
    })),
}
```

**Impact**: Form submissions can't flow through transform middleware

---

### Multipart Form Data ❌

**Content-Type**: `multipart/form-data; boundary=----WebKitFormBoundary...`

**Current Behavior**: **NOT parsed**
- ❌ Boundary not detected
- ❌ Parts not extracted
- ❌ File uploads not parsed
- ❌ `normalized_data = None`

**Example:**
```http
POST /upload HTTP/1.1
Content-Type: multipart/form-data; boundary=----WebKitFormBoundary7MA4YWxkTrZu0gW

------WebKitFormBoundary7MA4YWxkTrZu0gW
Content-Disposition: form-data; name="title"

My Photo
------WebKitFormBoundary7MA4YWxkTrZu0gW
Content-Disposition: form-data; name="file"; filename="photo.jpg"
Content-Type: image/jpeg

<binary data>
------WebKitFormBoundary7MA4YWxkTrZu0gW--
```

**Current Envelope:**
```rust
RequestEnvelope {
    original_data: b'------WebKitFormBoundary7MA4YWxkTrZu0gW\r\n...',
    normalized_data: None,  // ← NOT PARSED
}
```

**Expected Envelope** (with parsing):
```rust
RequestEnvelope {
    original_data: b'------WebKit...',
    normalized_data: Some(Value::Object({
        "fields": {
            "title": "My Photo"
        },
        "files": [{
            "name": "file",
            "filename": "photo.jpg",
            "content_type": "image/jpeg",
            "size": 12345,
            "path": "/tmp/upload_abc123"  // Saved to temp location
        }]
    })),
}
```

**Impact**: File uploads must be handled in custom middleware

---

### Binary Requests (Images, PDFs, etc.) ⚠️

**Content-Type**: `image/jpeg`, `image/png`, `application/pdf`, `application/octet-stream`

**Current Behavior**: **Partially works**
- ✅ `original_data` contains raw bytes
- ✅ `normalized_data = None` (correct - shouldn't parse)
- ⚠️ No metadata extraction (size, type, etc.)

**Example:**
```http
POST /upload/photo HTTP/1.1
Content-Type: image/jpeg

<binary JPEG data>
```

**Current Envelope:**
```rust
RequestEnvelope {
    original_data: vec![0xFF, 0xD8, 0xFF, ...],  // JPEG bytes
    normalized_data: None,  // ← Correct for binary
}
```

**Enhanced Envelope** (with metadata):
```rust
RequestEnvelope {
    original_data: vec![0xFF, 0xD8, ...],
    normalized_data: Some(Value::Object({
        "content_type": "image/jpeg",
        "size": 12345,
        "encoding": "binary",
        "checksum": "sha256:abc123..."
    })),
}
```

**Impact**: Works for passthrough, but lacks metadata for routing/validation

---

## Header Handling Analysis

### What's Captured ✅

All headers are converted to a string map in `ProtocolCtx.attrs.headers`:

```rust
// src/adapters/http/mod.rs:62-74
let headers_obj: serde_json::Value = {
    let map: serde_json::Map<String, serde_json::Value> = req
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                serde_json::Value::String(v.to_str().unwrap_or_default().to_string()),
            )
        })
        .collect();
    serde_json::Value::Object(map)
};
```

**Behavior:**
- ✅ All headers preserved
- ✅ Case-insensitive keys (converted to lowercase by HTTP library)
- ✅ Multi-value headers collapsed to first value
- ⚠️ Binary header values converted to empty string (`.unwrap_or_default()`)

### What's Missing ❌

1. **Multi-value header preservation**
   - Current: Only first value captured
   - Example: `Accept: application/json, text/xml` → `"application/json"`
   
2. **Content-Type parsing**
   - Current: Raw string `"multipart/form-data; boundary=xyz"`
   - Missing: Parsed components (media type, charset, boundary)

3. **Content-Encoding handling**
   - Current: Header captured but not used
   - Missing: Automatic decompression (gzip, deflate, br)

4. **Character encoding detection**
   - Current: Assumes UTF-8
   - Missing: Charset parameter parsing from Content-Type

---

## Comparison: FHIR vs HTTP Service

### FHIR Service (Better Content-Type Handling)

**Response handling** (`src/models/services/types/fhir.rs:214-229`):
```rust
if let Some(content_type) = response_envelope
    .response_details
    .headers
    .get("content-type")
{
    if content_type.contains("application/fhir+json")
        || content_type.contains("application/json")
    {
        if let Ok(json_value) =
            serde_json::from_slice::<serde_json::Value>(&response_envelope.original_data)
        {
            response_envelope.normalized_data = Some(json_value);
        }
    }
}
```

**Differences:**
- ✅ Checks for **multiple content types** (`fhir+json` OR `json`)
- ✅ Explicitly sets FHIR headers (`application/fhir+json`)
- ⚠️ Still JSON-only (no XML FHIR support)

### HTTP Service (Minimal Content-Type Handling)

**Request handling** (`src/models/services/types/http.rs:158-159`):
```rust
let normalized_data = serde_json::from_slice(&ctx.payload).ok();
```

**Differences:**
- ❌ No Content-Type inspection
- ❌ Always attempts JSON parse
- ❌ Silently fails for non-JSON

---

## Recommendations

### Phase 1: Content-Type-Aware Parsing (Critical)

**Location**: `src/adapters/http/mod.rs::http_request_to_protocol_ctx()`

**Add content-type inspection before body parsing:**

```rust
// Extract and parse Content-Type header
let content_type = req
    .headers()
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("application/octet-stream");

// Parse media type components
let (media_type, charset, boundary) = parse_content_type(content_type);

// Read body bytes
let body_bytes = axum::body::to_bytes(
    std::mem::replace(req.body_mut(), Body::empty()), 
    usize::MAX
).await?.to_vec();

// Parse body based on content type
let (normalized_data, parsed_metadata) = match media_type {
    "application/json" | "application/fhir+json" | "*/*+json" => {
        // JSON parsing (existing behavior)
        let json = serde_json::from_slice(&body_bytes).ok();
        (json, None)
    }
    
    "application/xml" | "text/xml" | "application/soap+xml" => {
        // XML parsing (NEW)
        let xml_handler = XmlHandler::new();
        match xml_handler.from_xml(&body_bytes) {
            Ok(json) => (Some(json), Some(json!({
                "format": "xml",
                "charset": charset
            }))),
            Err(_) => (None, Some(json!({
                "format": "xml",
                "parse_error": true
            })))
        }
    }
    
    "text/csv" => {
        // CSV parsing (NEW)
        let csv_handler = CsvHandler::new();
        match csv_handler.parse_csv(&body_bytes, true) {
            Ok(rows) => (Some(json!({ "rows": rows })), Some(json!({
                "format": "csv",
                "row_count": rows.len()
            }))),
            Err(_) => (None, Some(json!({
                "format": "csv",
                "parse_error": true
            })))
        }
    }
    
    "application/x-www-form-urlencoded" => {
        // Form data parsing (NEW)
        match parse_form_urlencoded(&body_bytes) {
            Ok(form_data) => (Some(form_data), Some(json!({
                "format": "form"
            }))),
            Err(_) => (None, Some(json!({
                "format": "form",
                "parse_error": true
            })))
        }
    }
    
    "multipart/form-data" => {
        // Multipart parsing (NEW)
        match parse_multipart(&body_bytes, boundary) {
            Ok((fields, files)) => (Some(json!({
                "fields": fields,
                "files": files
            })), Some(json!({
                "format": "multipart",
                "file_count": files.len()
            }))),
            Err(_) => (None, Some(json!({
                "format": "multipart",
                "parse_error": true
            })))
        }
    }
    
    _ if media_type.starts_with("image/") || 
         media_type.starts_with("video/") ||
         media_type.starts_with("audio/") => {
        // Binary content (images, videos, audio)
        (None, Some(json!({
            "format": "binary",
            "content_type": media_type,
            "size": body_bytes.len(),
            "checksum": calculate_checksum(&body_bytes)
        })))
    }
    
    _ => {
        // Unknown/unsupported content type
        // Try JSON as fallback (existing behavior)
        let json = serde_json::from_slice(&body_bytes).ok();
        (json, Some(json!({
            "format": "unknown",
            "content_type": media_type
        })))
    }
};

// Store parsed metadata in ProtocolCtx
let mut meta_map = HashMap::new();
meta_map.insert("protocol".to_string(), "http".to_string());
meta_map.insert("content_type".to_string(), media_type.to_string());
if let Some(parsed_meta) = parsed_metadata {
    meta_map.insert("parsed_metadata".to_string(), parsed_meta.to_string());
}
```

---

### Phase 2: Helper Functions

**Location**: `src/adapters/http/content_type.rs` (new file)

```rust
/// Parse Content-Type header into components
pub fn parse_content_type(header: &str) -> (String, Option<String>, Option<String>) {
    let parts: Vec<&str> = header.split(';').map(|s| s.trim()).collect();
    let media_type = parts[0].to_lowercase();
    
    let mut charset = None;
    let mut boundary = None;
    
    for part in parts.iter().skip(1) {
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().trim_matches('"');
            
            match key.as_str() {
                "charset" => charset = Some(value.to_string()),
                "boundary" => boundary = Some(value.to_string()),
                _ => {}
            }
        }
    }
    
    (media_type, charset, boundary)
}

/// Parse application/x-www-form-urlencoded body
pub fn parse_form_urlencoded(body: &[u8]) -> Result<serde_json::Value, Error> {
    let body_str = std::str::from_utf8(body)
        .map_err(|_| Error::from("Invalid UTF-8 in form data"))?;
    
    let mut result = serde_json::Map::new();
    
    for (key, value) in url::form_urlencoded::parse(body_str.as_bytes()) {
        // Handle array notation: field[] or field[0]
        if key.ends_with("[]") {
            let key_base = key.trim_end_matches("[]");
            result
                .entry(key_base)
                .or_insert_with(|| serde_json::Value::Array(vec![]))
                .as_array_mut()
                .unwrap()
                .push(serde_json::Value::String(value.to_string()));
        } else {
            result.insert(key.to_string(), serde_json::Value::String(value.to_string()));
        }
    }
    
    Ok(serde_json::Value::Object(result))
}

/// Parse multipart/form-data body
pub fn parse_multipart(
    body: &[u8], 
    boundary: Option<String>
) -> Result<(serde_json::Value, Vec<FileUpload>), Error> {
    use multer::Multipart;
    
    let boundary = boundary.ok_or_else(|| Error::from("Missing boundary in multipart"))?;
    
    // Use multer crate for parsing
    // Returns (fields, files)
    todo!("Implement multipart parsing with multer crate")
}

/// Calculate SHA256 checksum for binary data
pub fn calculate_checksum(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
```

---

### Phase 3: Format Handler Registry

**Location**: `src/formats/registry.rs` (new file)

```rust
use std::collections::HashMap;
use serde_json::Value;

pub trait FormatHandler: Send + Sync {
    fn name(&self) -> &str;
    fn media_types(&self) -> Vec<&str>;
    fn parse(&self, data: &[u8]) -> Result<Value, Box<dyn std::error::Error>>;
    fn serialize(&self, data: &Value) -> Result<Vec<u8>, Box<dyn std::error::Error>>;
}

pub struct FormatRegistry {
    handlers: HashMap<String, Box<dyn FormatHandler>>,
}

impl FormatRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            handlers: HashMap::new(),
        };
        
        // Register built-in handlers
        registry.register(Box::new(JsonHandler));
        registry.register(Box::new(XmlHandler));
        registry.register(Box::new(CsvHandler));
        
        registry
    }
    
    pub fn register(&mut self, handler: Box<dyn FormatHandler>) {
        for media_type in handler.media_types() {
            self.handlers.insert(media_type.to_string(), handler.clone());
        }
    }
    
    pub fn get(&self, media_type: &str) -> Option<&Box<dyn FormatHandler>> {
        self.handlers.get(media_type)
    }
}

// Global registry instance
lazy_static! {
    static ref FORMAT_REGISTRY: FormatRegistry = FormatRegistry::new();
}
```

**Usage in HttpAdapter:**
```rust
let handler = FORMAT_REGISTRY.get(media_type);
let normalized_data = handler
    .map(|h| h.parse(&body_bytes).ok())
    .flatten();
```

---

### Phase 4: Enhanced Envelope Metadata

**Location**: `src/models/envelope/envelope.rs`

Add `ContentMetadata` to RequestDetails:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContentMetadata {
    pub content_type: String,
    pub charset: Option<String>,
    pub content_length: Option<usize>,
    pub content_encoding: Option<String>,
    pub boundary: Option<String>,  // For multipart
    pub format: String,  // "json", "xml", "csv", "form", "multipart", "binary"
    pub parse_status: ParseStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ParseStatus {
    Success,
    Failed { reason: String },
    NotAttempted,
    Unsupported,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestDetails {
    pub method: String,
    pub uri: String,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub query_params: HashMap<String, Vec<String>>,
    pub cache_status: Option<String>,
    pub metadata: HashMap<String, String>,
    
    // NEW: Content metadata
    pub content_metadata: Option<ContentMetadata>,
}
```

---

## Dependencies Required

### Crates to Add

```toml
[dependencies]
# XML parsing
quick-xml = "0.31"
serde-xml-rs = "0.6"

# CSV parsing
csv = "1.3"

# Multipart form data
multer = "3.0"

# Character encoding detection
encoding_rs = "0.8"

# Checksum calculation
sha2 = "0.10"

# Lazy static for registry
lazy_static = "1.4"
```

---

## Impact on Pipeline Types

### Pipeline #1: CSV to JSON/XML ✅

**Before**: ❌ CSV not parsed, manual handling required  
**After**: ✅ Automatic CSV → JSON normalization

### Pipeline #3: Webhook to Database ✅

**Before**: ⚠️ Works for JSON webhooks only  
**After**: ✅ Supports form-data webhooks

### Pipeline #10: SOAP to REST ✅

**Before**: ❌ XML not parsed, SOAP envelope opaque  
**After**: ✅ Automatic XML → JSON normalization

### Binary/File Upload Pipelines ✅

**Before**: ⚠️ Works but no metadata  
**After**: ✅ File metadata available for routing/validation

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_json_request_parsing() {
        let req = build_test_request(
            "POST", 
            "/api/resource",
            "application/json",
            br#"{"name": "Alice"}"#
        );
        
        let ctx = HttpAdapter::http_request_to_protocol_ctx(&mut req, &HashMap::new())
            .await
            .unwrap();
        
        let envelope = HttpEndpoint.build_protocol_envelope(ctx, &HashMap::new())
            .await
            .unwrap();
        
        assert!(envelope.normalized_data.is_some());
        assert_eq!(envelope.normalized_data.unwrap()["name"], "Alice");
    }
    
    #[tokio::test]
    async fn test_xml_request_parsing() {
        let req = build_test_request(
            "POST",
            "/api/resource",
            "application/xml",
            b"<person><name>Alice</name></person>"
        );
        
        let ctx = HttpAdapter::http_request_to_protocol_ctx(&mut req, &HashMap::new())
            .await
            .unwrap();
        
        let envelope = HttpEndpoint.build_protocol_envelope(ctx, &HashMap::new())
            .await
            .unwrap();
        
        assert!(envelope.normalized_data.is_some());
        assert_eq!(envelope.normalized_data.unwrap()["name"], "Alice");
    }
    
    #[tokio::test]
    async fn test_csv_request_parsing() {
        let req = build_test_request(
            "POST",
            "/upload/csv",
            "text/csv",
            b"name,age\nAlice,30\nBob,25"
        );
        
        let ctx = HttpAdapter::http_request_to_protocol_ctx(&mut req, &HashMap::new())
            .await
            .unwrap();
        
        let envelope = HttpEndpoint.build_protocol_envelope(ctx, &HashMap::new())
            .await
            .unwrap();
        
        assert!(envelope.normalized_data.is_some());
        let rows = envelope.normalized_data.unwrap()["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
    }
    
    #[tokio::test]
    async fn test_form_urlencoded_parsing() {
        let req = build_test_request(
            "POST",
            "/form",
            "application/x-www-form-urlencoded",
            b"name=Alice&age=30&interests%5B%5D=coding"
        );
        
        let ctx = HttpAdapter::http_request_to_protocol_ctx(&mut req, &HashMap::new())
            .await
            .unwrap();
        
        let envelope = HttpEndpoint.build_protocol_envelope(ctx, &HashMap::new())
            .await
            .unwrap();
        
        assert!(envelope.normalized_data.is_some());
        assert_eq!(envelope.normalized_data.unwrap()["name"], "Alice");
    }
    
    #[tokio::test]
    async fn test_binary_request_no_parsing() {
        let req = build_test_request(
            "POST",
            "/upload/photo",
            "image/jpeg",
            &[0xFF, 0xD8, 0xFF, 0xE0]  // JPEG header
        );
        
        let ctx = HttpAdapter::http_request_to_protocol_ctx(&mut req, &HashMap::new())
            .await
            .unwrap();
        
        let envelope = HttpEndpoint.build_protocol_envelope(ctx, &HashMap::new())
            .await
            .unwrap();
        
        // Binary content should NOT be parsed
        assert!(envelope.normalized_data.is_none());
        assert_eq!(envelope.original_data.len(), 4);
        
        // But metadata should be populated
        assert!(envelope.request_details.content_metadata.is_some());
        let meta = envelope.request_details.content_metadata.unwrap();
        assert_eq!(meta.format, "binary");
        assert_eq!(meta.content_type, "image/jpeg");
    }
}
```

---

## Migration Path

### Backward Compatibility

**Existing behavior preserved for JSON requests:**
- ✅ No changes to JSON parsing logic
- ✅ Existing pipelines continue to work
- ✅ `normalized_data` population unchanged for JSON

**New behavior for non-JSON:**
- ✅ Opt-in via Content-Type header
- ✅ Fallback to existing behavior if parsing fails
- ✅ No breaking changes to Envelope structure

### Rollout Strategy

1. **Phase 1 (Week 1)**: Add content-type parsing infrastructure
   - Format handler interface
   - Content-type parser
   - Basic XML/CSV handlers

2. **Phase 2 (Week 2)**: Integrate into HttpAdapter
   - Update `http_request_to_protocol_ctx()`
   - Add comprehensive tests
   - Update documentation

3. **Phase 3 (Week 3)**: Add advanced handlers
   - Form data parsing
   - Multipart support
   - Binary metadata extraction

4. **Phase 4 (Week 4)**: Production validation
   - Integration tests with real pipelines
   - Performance benchmarks
   - Security review

---

## Performance Considerations

### Memory Impact

**Current**: Single copy of body bytes in `ProtocolCtx.payload`

**Proposed**: 
- Body bytes in `original_data` (unchanged)
- Parsed representation in `normalized_data` (new, but optional)
- **Memory overhead**: ~2x for parsed requests (JSON, XML, CSV)
- **Mitigation**: Keep `original_data` as primary, `normalized_data` as view

### Parsing Overhead

| Format | Parse Time (1MB) | Memory Overhead |
|--------|------------------|-----------------|
| JSON | ~5ms | 1.5-2x |
| XML | ~8ms | 2-3x |
| CSV | ~3ms | 1.2-1.5x |
| Form | ~1ms | 1.1x |
| Binary | 0ms (metadata only) | 1.01x |

**Mitigation**: 
- Lazy parsing (only when middleware needs `normalized_data`)
- Streaming for large bodies (>10MB)
- Configurable size limits per content type

---

## Security Considerations

### XML External Entity (XXE) Prevention

```rust
// Use safe XML parser configuration
let xml_config = quick_xml::Config {
    enable_dtd: false,  // Disable DTD processing
    enable_external_entities: false,  // Block external entities
    ..Default::default()
};
```

### CSV Injection Prevention

```rust
// Sanitize CSV cells to prevent formula injection
fn sanitize_csv_cell(cell: &str) -> String {
    if cell.starts_with('=') || cell.starts_with('+') || 
       cell.starts_with('-') || cell.starts_with('@') {
        format!("'{}", cell)  // Prefix with single quote
    } else {
        cell.to_string()
    }
}
```

### Size Limits

```toml
[proxy]
max_body_size = 10485760  # 10MB default
max_csv_rows = 10000
max_xml_depth = 50
max_multipart_files = 10
max_form_fields = 100
```

---

## Summary

**Current State**: HTTP endpoint is **JSON-centric** with limited content-type awareness

**Limitations**:
- ❌ No CSV parsing
- ❌ No XML parsing
- ❌ No form data parsing
- ❌ No multipart handling
- ⚠️ Binary content works but lacks metadata

**Recommended Solution**: **Content-Type-Aware Parsing Layer**
- ✅ Pluggable format handlers
- ✅ Backward compatible
- ✅ Minimal performance impact
- ✅ Enables 4+ pipeline types

**Implementation Timeline**: 4 weeks
**Estimated Effort**: ~2,000 LOC + tests

---

**Next Steps**:
1. Review and approve architecture
2. Prototype XML/CSV handlers (Week 1)
3. Integrate into HttpAdapter (Week 2)
4. Add advanced formats (Week 3)
5. Production validation (Week 4)
