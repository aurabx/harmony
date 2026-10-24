---
title: Multi-Content-Type Support
description: How Harmony parses JSON, XML, CSV, form data, multipart, and binary bodies.
status: active
updated: 2026-10-24
type: Configuration
---

# Multi-Content-Type Support

Harmony proxy supports automatic parsing of multiple content types beyond JSON, enabling seamless data transformation across different formats in healthcare and data integration pipelines.

## Overview

The HTTP adapter automatically detects and parses incoming request bodies based on the `Content-Type` header, converting them to a normalized JSON structure for pipeline processing. This enables middleware and backends to work with a consistent data model regardless of the original format.

## Supported Content Types

How each supported content type is parsed into normalised data. See [Supported Content Types](content-types-formats.md).

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

## Content Type Security and Error Handling

Security limits, error handling, fallback behaviour, and troubleshooting for content parsing. See [Content Type Security and Error Handling](content-types-handling.md).

## Examples

See `examples/content-types/` directory for:
- Sample configuration files
- Example requests for each content type
- JOLT transforms for format conversion
- Integration test examples

## Related Documentation

- [Endpoints Guide](endpoints.md) - Endpoint configuration
- [Middleware Guide](middleware.md) - Processing middleware
- [Security Guide](../security/overview.md) - Security best practices
- [Configuration Reference](configuration-file.md) - Complete config options
