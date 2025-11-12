# Multi-Content-Type Support

Harmony proxy supports automatic parsing of multiple content types beyond JSON, enabling seamless data transformation across different formats in healthcare and data integration pipelines.

## Overview

The HTTP adapter automatically detects and parses incoming request bodies based on the `Content-Type` header, converting them to a normalized JSON structure for pipeline processing. This enables middleware and backends to work with a consistent data model regardless of the original format.

## Supported Content Types

### 1. JSON (Default)

**Content-Type Headers:**
- `application/json`
- `application/fhir+json` (FHIR resources)
- `application/dicom+json` (DICOM JSON)

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/json" \
  -d '{"name": "Alice", "age": 30}'
```

**Normalized Structure:**
```json
{
  "name": "Alice",
  "age": 30
}
```

**Notes:**
- Default format when Content-Type is missing or unrecognized
- Direct pass-through to normalized_data
- Validates JSON syntax

### 2. XML

**Content-Type Headers:**
- `application/xml`
- `text/xml`
- `application/soap+xml`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/xml" \
  -d '<person><name>Bob</name><age>25</age></person>'
```

**Normalized Structure:**
```json
{
  "person": {
    "name": "Bob",
    "age": "25"
  }
}
```

**XML Features:**
- **Text-only elements**: Converted to simple string values
- **Attributes**: Prefixed with `@` (e.g., `"@id": "123"`)
- **Nested elements**: Preserved as nested objects
- **Multiple elements with same name**: Converted to arrays
- **Mixed content**: Text stored in `#text` field when attributes present

**Example with Attributes:**
```xml
<person id="123" type="customer">
  <name>Charlie</name>
</person>
```

Becomes:
```json
{
  "person": {
    "@id": "123",
    "@type": "customer",
    "name": "Charlie"
  }
}
```

**Security:** XXE (XML External Entity) attacks are prevented - quick-xml does not support external entities by default.

### 3. CSV

**Content-Type Header:**
- `text/csv`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: text/csv" \
  -d 'name,age,city
Alice,30,NYC
Bob,25,LA'
```

**Normalized Structure:**
```json
{
  "rows": [
    {"name": "Alice", "age": "30", "city": "NYC"},
    {"name": "Bob", "age": "25", "city": "LA"}
  ],
  "row_count": 2
}
```

**CSV Features:**
- First row treated as header
- All values parsed as strings
- Empty fields supported
- Handles quoted fields with commas

**Security:** Formula injection prevention - fields starting with `=`, `+`, `-`, or `@` are automatically prefixed with a single quote (`'`) to prevent execution in spreadsheet applications.

Example:
```csv
name,formula
Alice,=SUM(A1:A10)
```

Becomes:
```json
{
  "rows": [
    {"name": "Alice", "formula": "'=SUM(A1:A10)"}
  ]
}
```

### 4. Form URL-Encoded

**Content-Type Header:**
- `application/x-www-form-urlencoded`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d 'name=Alice&age=30&city=NYC'
```

**Normalized Structure:**
```json
{
  "name": "Alice",
  "age": "30",
  "city": "NYC"
}
```

**Array Support:**
Use `[]` notation for arrays:
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d 'name=Alice&interests[]=coding&interests[]=music'
```

Becomes:
```json
{
  "name": "Alice",
  "interests": ["coding", "music"]
}
```

### 5. Multipart Form Data

**Content-Type Header:**
- `multipart/form-data; boundary=<boundary>`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/upload \
  -F "name=Alice" \
  -F "age=30" \
  -F "file=@document.pdf"
```

**Normalized Structure:**
```json
{
  "fields": {
    "name": "Alice",
    "age": "30"
  },
  "files": [
    {
      "name": "file",
      "filename": "document.pdf",
      "content_type": "application/pdf",
      "size": 12345,
      "checksum": "a1b2c3d4..."
    }
  ]
}
```

**File Handling:**
- Files are NOT saved to disk automatically
- File metadata captured for pipeline processing
- SHA256 checksum computed for integrity verification
- Middleware/backends can access file data via envelope

### 6. Binary Content

**Content-Type Headers:**
- `image/*` (JPEG, PNG, GIF, etc.)
- `video/*`
- `audio/*`
- `application/pdf`
- `application/zip`
- `application/octet-stream`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/upload \
  -H "Content-Type: image/jpeg" \
  --data-binary @photo.jpg
```

**Normalized Structure:**
```json
{
  "format": "binary",
  "content_type": "image/jpeg",
  "size": 45678,
  "checksum": "abc123..."
}
```

**Notes:**
- Binary data preserved in `original_data` field of envelope
- Checksum allows integrity verification
- Middleware can process binary data directly

## Configuration

### Content Limits

Configure size and complexity limits to prevent resource exhaustion:

```toml
[proxy.content_limits]
max_body_size = 10485760      # 10MB maximum request body
max_csv_rows = 10000           # Maximum CSV rows to parse
max_xml_depth = 100            # Maximum XML nesting depth
max_multipart_files = 10       # Maximum files in multipart upload
max_form_fields = 1000         # Maximum form fields
```

**Defaults:**
- `max_body_size`: 10MB (10,485,760 bytes)
- `max_csv_rows`: 10,000 rows
- `max_xml_depth`: 100 levels
- `max_multipart_files`: 10 files
- `max_form_fields`: 1,000 fields

### Example Configuration

```toml
[proxy]
id = "content-aware-proxy"
store_dir = "./data"

[proxy.content_limits]
max_body_size = 20971520  # 20MB for larger uploads
max_csv_rows = 50000      # Support larger CSV files
max_xml_depth = 50        # Limit XML complexity

[logging]
log_level = "info"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "0.0.0.0"
bind_port = 8080

[pipelines.api]
description = "Multi-format API pipeline"
networks = ["default"]
endpoints = ["api_endpoint"]
backends = ["processing_backend"]
middleware = []

[endpoints.api_endpoint]
service = "http"
[endpoints.api_endpoint.options]
path_prefix = "/api"

[backends.processing_backend]
service = "http"
[backends.processing_backend.options]
base_url = "http://backend-service:8080"

[services.http]
module = ""
```

## Content Metadata

Each request includes content metadata in the envelope for tracking parsing status and format details:

```rust
pub struct ContentMetadata {
    pub content_type: String,      // Original Content-Type header
    pub charset: Option<String>,    // Character encoding if specified
    pub format: String,             // Detected format: json, xml, csv, etc.
    pub parse_status: ParseStatus,  // Success, Failed, NotAttempted, Unsupported
    pub original_size: usize,       // Size of original payload in bytes
    pub checksum: Option<String>,   // SHA256 checksum (for binary content)
}
```

**Parse Status Values:**
- `Success`: Content parsed successfully
- `Failed`: Parsing attempted but failed (malformed data)
- `NotAttempted`: No parsing attempted (empty payload)
- `Unsupported`: Content-Type not supported

**Accessing in Middleware:**
```rust
async fn process(envelope: RequestEnvelope<Vec<u8>>) -> Result<RequestEnvelope<Vec<u8>>, Error> {
    if let Some(metadata) = &envelope.request_details.content_metadata {
        tracing::info!(
            "Processing {} content ({}), parse_status: {:?}",
            metadata.format,
            metadata.content_type,
            metadata.parse_status
        );
    }
    Ok(envelope)
}
```

## Security Considerations

### XXE Prevention (XML)

**Threat:** XML External Entity (XXE) attacks allow attackers to read local files or perform SSRF attacks.

**Mitigation:** The quick-xml parser does not support external entities by default. External entity declarations are ignored.

**Example Attack (Blocked):**
```xml
<!DOCTYPE foo [
  <!ENTITY xxe SYSTEM "file:///etc/passwd">
]>
<data>&xxe;</data>
```

This will parse as if the entity doesn't exist, preventing file disclosure.

### Formula Injection Prevention (CSV)

**Threat:** CSV formula injection occurs when spreadsheet applications execute formulas in CSV cells.

**Mitigation:** Fields starting with `=`, `+`, `-`, or `@` are automatically prefixed with a single quote.

**Before:**
```csv
name,command
Alice,=cmd|'/c calc'!A1
```

**After Parsing:**
```json
{"name": "Alice", "command": "'=cmd|'/c calc'!A1"}
```

### XML Bomb Prevention

**Threat:** Billion Laughs attack (XML bomb) causes exponential entity expansion.

**Mitigation:** 
- Maximum XML depth limit (default: 100)
- No entity expansion support
- Maximum body size limit (default: 10MB)

### Multipart Security

**Threats:**
- Path traversal via malicious filenames
- Resource exhaustion via many small files
- Memory exhaustion via large files

**Mitigations:**
- Filename sanitization (automatic by multer)
- Maximum file count limit (default: 10)
- Maximum body size limit (default: 10MB)
- Files not automatically written to disk

### Size Limits

All content types respect the `max_body_size` limit. Additional per-format limits:

- **CSV**: Row count limit prevents memory exhaustion
- **XML**: Depth limit prevents stack overflow
- **Multipart**: File count limit prevents descriptor exhaustion
- **Form**: Field count limit prevents hash collision attacks

## Error Handling

### Malformed Content

When content parsing fails:
1. `parse_status` set to `Failed`
2. Warning logged with error details
3. `normalized_data` set to `None`
4. Pipeline continues with `original_data` available
5. Middleware can check `parse_status` and handle accordingly

Example log:
```
WARN harmony: Failed to parse XML: XML parsing error: unexpected EOF
```

### Unsupported Content-Type

When Content-Type is unknown:
1. Attempts to parse as JSON (fallback behavior)
2. If JSON parsing fails, `parse_status` set to `Unsupported`
3. Request continues through pipeline
4. Backend receives raw data in `original_data`

### Size Limit Exceeded

When limits are exceeded:
1. Parsing terminates immediately
2. Error returned to client (400 Bad Request)
3. Descriptive error message includes limit value

Example error:
```json
{
  "error": "CSV row count exceeds limit of 10000"
}
```

## Fallback Behavior

**Missing Content-Type:** Defaults to `application/json`

**Unknown Content-Type:** 
1. Attempts JSON parsing
2. If JSON parsing fails, marks as `Unsupported`
3. Pipeline continues with raw data

**Empty Payload:** 
- `parse_status` set to `NotAttempted`
- `normalized_data` set to `None`
- No parsing attempted

## Best Practices

### 1. Always Set Content-Type

Explicitly set the Content-Type header to ensure correct parsing:

```bash
# Good
curl -H "Content-Type: text/csv" -d @data.csv http://...

# Avoid (will try JSON parsing)
curl -d @data.csv http://...
```

### 2. Validate in Middleware

Check parse status in middleware before processing:

```rust
if envelope.request_details.content_metadata
    .as_ref()
    .map_or(false, |m| m.parse_status != ParseStatus::Success) 
{
    return Err(Error::from("Content parsing failed"));
}
```

### 3. Configure Appropriate Limits

Set limits based on your use case:

- **Small API endpoints**: Lower limits (1MB, 100 rows)
- **File upload services**: Higher limits (100MB, more files)
- **Untrusted inputs**: Conservative limits
- **Internal services**: Relaxed limits

### 4. Monitor Parse Failures

Track parsing failures in logs and metrics:

```rust
if let Some(metadata) = &envelope.request_details.content_metadata {
    if metadata.parse_status != ParseStatus::Success {
        metrics::counter!("parse_failures", "format" => &metadata.format).increment(1);
    }
}
```

### 5. Use Binary Content for Large Files

For large files, use binary content types rather than base64-encoded JSON:

```bash
# Efficient
curl -H "Content-Type: application/pdf" --data-binary @large.pdf http://...

# Inefficient (33% overhead)
curl -H "Content-Type: application/json" -d '{"file":"<base64>"}' http://...
```

## Troubleshooting

### Issue: CSV not parsing correctly

**Symptoms:** CSV data appears in original_data but not in normalized_data

**Solutions:**
1. Verify `Content-Type: text/csv` header is set
2. Check CSV has valid header row
3. Ensure CSV is properly formatted (no unquoted commas in fields)
4. Check row count doesn't exceed `max_csv_rows` limit

### Issue: XML parsing fails

**Symptoms:** `parse_status: Failed` for XML content

**Solutions:**
1. Validate XML syntax with external tool
2. Check for unsupported features (external entities, DTDs)
3. Verify XML depth doesn't exceed `max_xml_depth` limit
4. Ensure proper UTF-8 encoding

### Issue: Multipart files not detected

**Symptoms:** Files array is empty in normalized_data

**Solutions:**
1. Verify `Content-Type: multipart/form-data` with boundary parameter
2. Ensure boundary in header matches boundary in body
3. Check that fields have `filename` attribute for file detection
4. Verify file count doesn't exceed `max_multipart_files` limit

### Issue: Request rejected with "exceeds limit"

**Symptoms:** 400 Bad Request with size limit error

**Solutions:**
1. Increase relevant limit in `[proxy.content_limits]` configuration
2. Split large requests into smaller chunks
3. Use streaming endpoints for very large files
4. Compress data before upload

## Performance Considerations

### Parsing Overhead

Approximate parsing overhead by content type:

- **JSON**: ~10-20μs for small payloads (<1KB)
- **XML**: ~50-100μs (includes structure conversion)
- **CSV**: ~100μs per 100 rows
- **Form URL-encoded**: ~20-30μs
- **Multipart**: ~500μs per file (includes checksum)
- **Binary**: ~1ms per MB (checksum calculation)

### Memory Usage

Memory overhead during parsing:

- **JSON**: ~1x payload size (serde_json)
- **XML**: ~2-3x payload size (DOM structure)
- **CSV**: ~2x payload size (row objects)
- **Multipart**: ~1.5x payload size (field buffers)

### Optimization Tips

1. **Use JSON when possible**: Fastest parsing, lowest overhead
2. **Stream large files**: Don't parse entire body if processing in chunks
3. **Disable checksums**: For trusted sources, skip checksum calculation
4. **Tune limits**: Set limits to actual use case requirements
5. **Monitor metrics**: Track parsing times and adjust limits

## Examples

See `examples/content-types/` directory for:
- Sample configuration files
- Example requests for each content type
- JOLT transforms for format conversion
- Integration test examples

## Related Documentation

- [Endpoints Guide](endpoints.md) - Endpoint configuration
- [Middleware Guide](middleware.md) - Processing middleware
- [Security Guide](security.md) - Security best practices
- [Configuration Reference](configuration.md) - Complete config options
