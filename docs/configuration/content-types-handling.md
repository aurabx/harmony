---
title: Content Type Security and Error Handling
description: Security limits, error handling, fallback behaviour, and troubleshooting for content parsing.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/content-types.md
---

# Content Type Security and Error Handling

Security limits, error handling, fallback behaviour, and troubleshooting for content parsing. This page is part of [Multi-Content-Type Support](content-types.md).

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
