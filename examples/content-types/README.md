# Multi-Content-Type Support Example

This example demonstrates Harmony's ability to parse and process multiple content types beyond JSON.

## Supported Content Types

Harmony automatically detects and parses the following content types based on the `Content-Type` header:

### 1. **JSON** (default)
- `application/json`
- `application/fhir+json`
- `application/dicom+json`
- Any `*/*+json` media type

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/json" \
  -d '{"name": "Alice", "age": 30}'
```

### 2. **XML**
- `application/xml`
- `text/xml`
- `application/soap+xml`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/xml" \
  -d '<person><name>Alice</name><age>30</age></person>'
```

**Parsed JSON Structure:**
```json
{
  "person": {
    "name": "Alice",
    "age": "30"
  }
}
```

### 3. **CSV**
- `text/csv`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: text/csv" \
  -d $'name,age,city\nAlice,30,NYC\nBob,25,LA'
```

**Parsed JSON Structure:**
```json
{
  "rows": [
    {"name": "Alice", "age": "30", "city": "NYC"},
    {"name": "Bob", "age": "25", "city": "LA"}
  ],
  "row_count": 2
}
```

### 4. **Form URL-Encoded**
- `application/x-www-form-urlencoded`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d 'name=Alice&age=30&interests[]=coding&interests[]=music'
```

**Parsed JSON Structure:**
```json
{
  "name": "Alice",
  "age": "30",
  "interests": ["coding", "music"]
}
```

### 5. **Multipart Form Data**
- `multipart/form-data`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/upload \
  -F "title=My Photo" \
  -F "file=@photo.jpg"
```

**Parsed JSON Structure:**
```json
{
  "fields": {
    "title": "My Photo"
  },
  "files": [
    {
      "name": "file",
      "filename": "photo.jpg",
      "content_type": "image/jpeg",
      "size": 12345,
      "checksum": "sha256:abc123..."
    }
  ]
}
```

### 6. **Binary Content**
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

**Parsed JSON Metadata:**
```json
{
  "format": "binary",
  "content_type": "image/jpeg",
  "size": 12345,
  "checksum": "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f"
}
```

## Configuration

### Content Limits

Configure size limits to prevent resource exhaustion:

```toml
[proxy.content_limits]
max_body_size = 10485760      # 10MB - Maximum request body size
max_csv_rows = 10000           # Maximum rows to parse from CSV files
max_xml_depth = 100            # Maximum XML nesting depth (prevents XML bombs)
max_multipart_files = 10       # Maximum files per multipart request
max_form_fields = 1000         # Maximum form fields per request
```

All limits are optional and have sensible defaults. If omitted, the defaults shown above are used.

## Content Metadata

Every request includes content metadata in the envelope:

```rust
pub struct ContentMetadata {
    pub content_type: String,        // Original Content-Type header
    pub charset: Option<String>,     // Character encoding (e.g., "utf-8")
    pub format: String,              // "json", "xml", "csv", "form", "multipart", "binary"
    pub parse_status: ParseStatus,   // Success, Failed, NotAttempted, Unsupported
    pub original_size: usize,        // Size in bytes
    pub checksum: Option<String>,    // SHA256 checksum (binary content only)
}
```

## Security Features

### XML External Entity (XXE) Prevention
- External entities are disabled by default in the XML parser
- No DTD processing
- Safe against XXE injection attacks

### CSV Formula Injection Prevention
- Fields starting with `=`, `+`, `-`, or `@` are automatically prefixed with `'`
- Prevents formula injection in spreadsheet applications

### Size Limits
- All parsers enforce configurable size limits
- Protects against:
  - XML bomb attacks (billion laughs)
  - CSV bombs (excessive rows)
  - Multipart bombs (excessive files)
  - Form data bombs (excessive fields)

### XML Depth Limits
- Maximum nesting depth of 100 (configurable)
- Prevents stack overflow from deeply nested XML

## Backward Compatibility

- Missing `Content-Type` header defaults to `application/json`
- Existing JSON-only pipelines work unchanged
- Content metadata is optional (`Option<ContentMetadata>`)
- Unknown content types fall back to JSON parsing

## Running the Example

1. Start Harmony with the example configuration:
```bash
cargo run -- --config examples/content-types/config.toml
```

2. Test with different content types (examples above)

3. Check the logs to see how content is parsed:
```bash
tail -f ./tmp/harmony.log
```

## Pipeline Integration

Parsed content is available in `normalized_data` for:
- JOLT transforms
- Middleware processing
- Backend routing decisions
- Custom middleware logic

Example transform middleware can convert between formats:
```toml
[middleware.xml_to_json]
type = "transform"
[middleware.xml_to_json.options]
spec_path = "transforms/xml_to_json.json"
apply = "left"  # Apply to request before backend
```

## Error Handling

When parsing fails:
- `normalized_data` is set to `None`
- `parse_status` is set to `Failed`
- Original bytes are preserved in `original_data`
- Error is logged with details
- Request continues through pipeline (graceful degradation)

## Troubleshooting

**Q: My XML isn't parsing**
- Check that Content-Type header is set correctly
- Verify XML is well-formed (no unclosed tags)
- Check logs for parsing errors
- Verify XML depth doesn't exceed limit (default: 100)

**Q: CSV parsing fails**
- Ensure first row contains headers
- Check for consistent column counts
- Verify row count doesn't exceed limit (default: 10,000)

**Q: Multipart upload not working**
- Verify boundary is included in Content-Type header
- Check file count doesn't exceed limit (default: 10)
- Ensure files aren't too large (check max_body_size)

**Q: How do I increase limits?**
- Add `[proxy.content_limits]` section to config
- Set desired limits (see Configuration section above)
- Restart Harmony to apply changes
