# Middleware

**Last Updated**: 2025-11-30

Middleware extends the request/response pipeline to authenticate, enrich, or transform the `RequestEnvelope` and `ResponseEnvelope` as they flow through the `PipelineExecutor`.

## Architecture

Middleware operates within the unified `PipelineExecutor` (see [router.md](router.md)) and processes envelopes for all protocols (HTTP, DIMSE, HL7, etc.).

```
RequestEnvelope → Incoming Middleware<RequestEnvelope> → Backend<ResponseEnvelope> → Outgoing Middleware<ResponseEnvelope>
```

**Middleware types**:
- **Authentication**: Should be run early in the pipeline (incoming side)
- **Transformation**: Can run on incoming requests ("left"), outgoing responses ("right"), or both
- **Path filtering**: Rejects requests based on URL patterns
- **Metadata transformation**: Modifies request metadata for routing decisions

**Key principle**: Middleware is protocol-agnostic. It works with envelopes, not raw protocol data.

## Error Handling

Incoming middleware errors are mapped to HTTP status codes as follows:
- **Authentication failures** (JWT/Basic auth credential problems): HTTP 401 Unauthorized
- **All other middleware failures** (transform errors, internal failures): HTTP 500 Internal Server Error

This ensures that only actual authentication problems result in 401 responses, while configuration errors, transform failures, and other internal issues correctly return 500.

## Authentication

### Basic Auth
Validates a `username`/`password` combination, typically supplied in the `Authorization: Basic <base64>` header.

Config keys:
- `username` (string)
- `password` (string)
- `token_path` (optional, string): file path for a pre-shared token, if used by your environment

Error handling: Authentication failures (missing/invalid credentials) return HTTP 401 Unauthorized.

Example:
```toml
[middleware.basic_auth_example]
type="basic_auth"
username = "test_user"
password = "test_password"
# token_path = "/tmp/test_token" # optional
```

### JWT Auth
Verification of `Authorization: Bearer <token>` using cryptographic signature checks and strict claims validation.

Supported modes:
- RS256 (default, recommended): Verify with an RSA public key in PEM format.
- HS256 (explicit, dev/test only): Verify with a symmetric secret when `use_hs256 = true`.

Behavior:
- Strict algorithm enforcement (no algorithm downgrades)
- Signature verified with `jsonwebtoken` crate
- Validates `exp`, `nbf`, and `iat` with optional leeway
- Validates `iss` and `aud` when configured
- Any verification error returns HTTP 401 Unauthorized
- Startup safety: if `use_hs256` is not explicitly set to true and no `public_key_path` is provided, the middleware will panic during initialization to avoid insecure defaults

Config keys:
- `public_key_path` (string, required for RS256): Path to RSA public key (PEM)
- `use_hs256` (bool, default false): Enable HS256 mode explicitly
- `hs256_secret` (string, required when `use_hs256 = true`): Shared secret for HS256
- `issuer` (string, optional): Expected `iss`
- `audience` (string, optional): Expected `aud`
- `leeway_secs` (integer, optional): Allowed clock skew when validating time-based claims

Examples
- RS256 (recommended):
```toml
[middleware.jwt_auth_example]
type = "jwt_auth"
public_key_path = "/etc/harmony/jwt_public.pem"
issuer = "https://auth.example.com/"
audience = "harmony"
leeway_secs = 60
```

- HS256 (development/test only):
```toml
[middleware.jwt_auth_example]
type = "jwt_auth"
use_hs256 = true
hs256_secret = "replace-with-strong-secret"
issuer = "https://auth.example.com/"
audience = "harmony"
leeway_secs = 60
```

Notes:
- Place JWT auth middleware early in your pipeline to reject unauthenticated requests before expensive work.
- Configuration parsing for this middleware lives within the middleware module itself.

Error handling: Authentication failures (missing/invalid/expired tokens) return HTTP 401 Unauthorized. Internal server errors (key parsing, configuration issues) return HTTP 500 Internal Server Error.

## Transformation

### Transform (JOLT)
Applies JSON-to-JSON transformations using JOLT specifications. Supports configurable application on request/response sides with error handling options.

**Cloud Integration**: When configurations are sourced from Runbeam Cloud, transform specifications (JOLT files) are automatically downloaded before the configuration is applied. The gateway:
- Extracts transform IDs from middleware `spec_path` fields
- Downloads JOLT specifications from Runbeam Cloud Transform API
- Writes transform files to the configured `transforms_path` directory
- Overwrites existing transforms with the same ID
- Fails the config change if any transform download fails

This ensures that all referenced transform specifications are available before pipeline execution begins. For manual configurations, transform files must be provided separately in the transforms directory.

## Path Filter

Filters incoming requests based on URL path patterns with explicit allow/deny rules. Uses first-match-wins evaluation with matchit pattern syntax.

### Configuration

Config keys:
- `rules` (array of objects, required): List of allow/deny rules, each with either an `allow` or `deny` key containing a matchit pattern

### Rule Structure

Each rule must be an object with either:
- `{ allow = "<pattern>" }` - Allow requests matching this pattern
- `{ deny = "<pattern>" }` - Deny requests matching this pattern

### Evaluation Logic

**First-match-wins**: Rules are processed in order from first to last. The first matching rule determines the outcome:
- **Allow rule matches**: Request continues to backend
- **Deny rule matches**: Middleware returns a `PathDenied` error; the HTTP adapter maps this to HTTP 404 Not Found and stops processing
- **No rule matches**: Implicit deny (middleware returns `PathDenied`, which the HTTP adapter maps to 404)

### Pattern Syntax

Supports matchit pattern syntax:
- **Exact paths**: `/users`, `/api/health`
- **Wildcards**: `/api/*` (matches `/api/anything`)
- **Catch-all**: `/{*rest}` (matches any path, use for deny/allow-all rules)
- **Parameters**: `/users/{id}` (matches `/users/123`, etc.)
- **Multiple segments**: `/api/{version}/users/{id}`

### Examples

**Allow specific paths, deny rest:**
```toml
[middleware.imagingstudy_filter]
type = "path_filter"
[middleware.imagingstudy_filter.options]
rules = [
  { allow = "/ImagingStudy" },
  { allow = "/Patient" },
  { deny = "/{*rest}" }  # Catch-all: deny all other paths
]
```

**Deny specific paths, allow rest:**
```toml
[middleware.api_filter]
type = "path_filter"
[middleware.api_filter.options]
rules = [
  { deny = "/admin/*" },     # Block admin paths first
  { deny = "/internal/*" },  # Block internal paths
  { allow = "/{*rest}" }      # Catch-all: allow everything else
]
```

**Complex mixed allow/deny rules:**
```toml
[middleware.complex_filter]
type = "path_filter"
[middleware.complex_filter.options]
rules = [
  { allow = "/api/public/*" },   # Allow public API (checked first)
  { deny = "/api/*" },            # Deny other API paths
  { allow = "/health" },          # Allow health check
  { allow = "/metrics" },         # Allow metrics endpoint
  { deny = "/{*rest}" }           # Catch-all: deny everything else
]
```

### Behavior

- Only applies to incoming requests (left side of middleware chain)
- Path matching uses the subpath after the endpoint's `path_prefix`
- Trailing slashes are normalized (e.g., "/ImagingStudy/" matches "/ImagingStudy")
- On denial: the middleware returns `PathDenied`; the HTTP adapter maps this to HTTP 404 with an empty body and does not invoke backends
- Empty path becomes "/" (root)

### Troubleshooting

**Order matters**: More specific patterns should come before broader patterns
- ✅ Good: `[{ allow = "/api/public/*" }, { deny = "/api/*" }]`
- ❌ Bad: `[{ deny = "/api/*" }, { allow = "/api/public/*" }]` (public never reached)

**Test incrementally**: Start with allow-all, then add specific denies (or vice versa)

**Use explicit deny-all**: Makes intent clear and prevents implicit deny surprises
```toml
rules = [
  { allow = "/path1" },
  { allow = "/path2" },
  { deny = "/{*rest}" }  # Catch-all: deny everything not allowed above
]
```

**Debug logging**: Set `RUST_LOG=harmony=debug` to see which rules match

## Metadata Transform

Applies JOLT transformations to request metadata (the HashMap&lt;String, String&gt; in RequestDetails). This allows dynamic modification of metadata fields that control backend behavior.

Config keys:
- `spec_path` (string, required): Path to JOLT specification file
- `apply` (string, optional): When to apply - "left", "right", or "both" (default: "left")
- `fail_on_error` (bool, optional): Whether to fail request on transform errors (default: true)

Example:
```toml
[middleware.fhir_dimse_meta]
type = "metadata_transform"
[middleware.fhir_dimse_meta.options]
spec_path = "transforms/metadata_set_dimse_op.json"
apply = "left"
fail_on_error = true
```

Behavior:
- Converts metadata to JSON object for JOLT processing
- Only string-valued outputs from JOLT are written back to metadata
- Preserves existing metadata fields not modified by transform
- Common use case: setting dimse_op field to control DICOM backend operations

## JMIX Builder

Builds JMIX envelopes from DICOM operation responses. Handles caching, indexing, and ZIP file creation for JMIX packages.

Config:
```toml
[middleware.jmix_builder]
type = "jmix_builder"
[middleware.jmix_builder.options]
# Performance optimization flags
skip_hashing = true   # Skip SHA256 hashing for faster processing
skip_listing = true   # Skip DICOM files from files.json manifest
```

**Configuration options:**
- `skip_hashing` (bool, optional, default: false): Skip SHA256 file hashing for faster processing
- `skip_listing` (bool, optional, default: false): Skip DICOM files from files.json manifest

**Left side behavior (request processing):**
- Processes GET/HEAD requests for JMIX endpoints (`/api/jmix/{id}`, `/api/jmix?studyInstanceUid=...`)
- Serves cached JMIX packages if they exist locally
- Returns manifest.json for manifest requests (`/api/jmix/{id}/manifest`)
- Sets `skip_backends=true` and response metadata when serving from cache
- Passes through to backends when no local package exists

**Right side behavior (response processing):**
- Detects DICOM "move"/"get" responses containing `folder_path` and `instances`
- Creates JMIX packages under storage using jmix-rs builder
- Copies DICOM files from the backend folder into the package payload
- Writes manifest.json and metadata.json files
- Creates ZIP files for distribution
- Indexes packages by StudyInstanceUID for query lookup
- Cleans up temporary DICOM files after successful ZIP creation

This middleware is typically used with JMIX endpoints that bridge to DICOM backends, automatically converting DICOM responses into distributable JMIX packages.

## DICOMweb Bridge

Bridges DICOMweb HTTP requests (QIDO-RS/WADO-RS) to DICOM operations and converts responses back to DICOMweb format.

Config:
```toml
[middleware.dicomweb_bridge]
type = "dicomweb_bridge"
```

**Left side behavior (DICOMweb → DICOM):**
- Maps DICOMweb URLs to DICOM operations:
  - `/studies` → C-FIND at study level
  - `/studies/{study}/series` → C-FIND at series level  
  - `/studies/{study}/series/{series}/instances` → C-FIND at instance level
  - `/studies/{study}/series/{series}/instances/{instance}` → C-GET (WADO) or C-FIND (QIDO)
  - `/studies/.../metadata` → C-FIND with full metadata
  - `/studies/.../frames/{frames}` → C-GET for frame extraction
- Converts query parameters to DICOM identifiers with hex tags
- Processes `includefield` parameter for attribute filtering
- Sets appropriate return keys based on query level and includefield
- Distinguishes between QIDO (JSON) and WADO (binary) based on Accept headers

**Right side behavior (DICOM → DICOMweb):**
- **QIDO responses**: Converts DICOM find results to DICOMweb JSON arrays
- **WADO metadata**: Returns filtered JSON metadata based on includefield
- **WADO instances**: Creates multipart/related responses with DICOM files
- **WADO frames**: Decodes DICOM pixel data to JPEG/PNG images
- Handles both single-frame and multi-frame responses
- Supports content negotiation (Accept: image/jpeg, image/png)
- Provides proper error responses for unsupported transfer syntaxes

**Features:**
- Full DICOMweb QIDO-RS and WADO-RS compliance
- Automatic DICOM tag name to hex conversion using dicom-rs StandardDataDictionary
- Support for multiple query parameter values
- Includefield filtering for bandwidth optimization
- Multipart response handling for bulk data
- Frame-level image extraction with format conversion

This middleware enables DICOMweb endpoints to communicate with traditional DICOM PACS systems via DIMSE protocols.

## DICOM Flatten

Flattens DICOM JSON structures for simplified processing, converting between standard DICOM JSON format (Part 18) and flat key-value pairs. Useful for downstream systems that require simplified DICOM data representation.

### Configuration

Config keys:
- `apply` (string, optional): When to apply - "left" (flatten requests), "right" (unflatten responses), or "both" (default)

### Example

```toml
[middleware.dicom_flatten_example]
type = "dicom_flatten"
[middleware.dicom_flatten_example.options]
apply = "both"  # Flatten on request, unflatten on response
```

### Behavior

**Flatten (left side):**
- Converts standard DICOM JSON with `{vr, Value}` structure to simple key-value pairs
- Tag ID → scalar value or nested structure for sequences
- Person Name (PN) VR: Extracts "Alphabetic" field
- Preserves VR metadata internally for reconstruction
- Example: `{"00100020": {"vr": "LO", "Value": ["PID123"]}}` → `{"00100020": "PID123"}`

**Unflatten (right side):**
- Reconstructs standard DICOM JSON from flattened form
- Uses preserved VR metadata to restore proper structure
- Recreates sequences (SQ) from nested arrays
- Restores Person Name structure with Alphabetic field
- Example: `{"00100020": "PID123"}` → `{"00100020": {"vr": "LO", "Value": ["PID123"]}}`

**Supported VR types:**
- Scalars: LO, UI, SH, DA, TM, DT, PN (with special handling)
- Sequences (SQ): Recursive flattening of nested items
- Multi-valued: Arrays preserved as-is
- Empty values: Handled as return keys

**Snapshot preservation:**
- Original data snapshot stored before transformation
- Enables inspection of pre-transformation state
- Compatible with debugging and log dump middleware

### Use Cases

- Simplify DICOM data for frontend/API consumption
- Bridge DICOM backends with systems expecting flat structures
- Round-trip transformations: flatten for processing, unflatten for DICOM compliance
- DICOM data export to simplified formats

## Log Dump

The log dump middleware outputs request or response envelopes to the logs to help builders and tools create and debug pipelines. It provides a comprehensive view of the current request/response state, including original and normalized data, making it particularly useful when placed after transformations.

### Configuration

Config keys:
- `apply` (string, optional): When to dump - "left", "right", or "both" (default: "both")
- `pretty` (bool, optional): Pretty print JSON output (default: true)
- `max_bytes` (integer, optional): Maximum bytes to include in logs for large content (default: 65536)
- `redact_headers` (array of strings, optional): Header names to redact (case-insensitive)
- `redact_metadata` (array of strings, optional): Metadata keys to redact
- `redact_data_fields` (array of strings, optional): Normalized data fields to redact (dot path notation)
- `label` (string, optional): Optional label to distinguish multiple dump points

### Example

```toml
[middleware.debug_dump]
type = "log_dump"
[middleware.debug_dump.options]
apply = "both"  # Dump both request and response
pretty = true   # Pretty print JSON
redact_headers = ["authorization", "cookie"]  # Redact sensitive headers
redact_metadata = ["api_key", "token"]     # Redact sensitive metadata
redact_data_fields = ["ssn", "password", "user.payment_details"]  # Redact normalized data fields
label = "after_transform"  # Help identify where in pipeline this occurred
max_bytes = 32768  # Limit for very large payloads
```

### Behavior

**Output includes**:
- Request/response details (method, URI, headers, cookies, query params, metadata)
- Normalized data and pre-transform snapshots (if available)
- Target details (backend routing information)
- Content metadata (format, parsing status, size)
- Configuration label and side indicator (left/right)

**Security considerations**:
- Always use redact options when dealing with production logs to avoid leaking PII or credentials
- Consider setting appropriate `max_bytes` to avoid log flooding
- Use `label` to help identify specific pipeline stages when using multiple dump points

**Log targeting**:
- All dump output uses the `harmony.dump` logging target
- Use `RUST_LOG=harmony.dump=info` to enable just dump logs (or `debug` for more verbose)
- Standard log filtering applies (can be directed to different files/destinations)

## Webhook

Sends pipeline request/response data to an external HTTP endpoint. Useful for audit logging, notifications, or external processing.

Config keys:
- `endpoint` (string, required): URL to POST data to
- `apply` (string, optional): When to apply - "left", "right", or "both" (default: "left")
- `redact_headers` (array of strings, optional): Header names to redact
- `redact_metadata` (array of strings, optional): Metadata keys to redact
- `timeout_secs` (integer, optional): Request timeout in seconds (default: 5)
- `authentication_def` (object, optional): Authentication configuration for the webhook request

Example:
```toml
[middleware.audit_webhook]
type = "webhook"
[middleware.audit_webhook.options]
endpoint = "https://audit.example.com/logs"
apply = "both"
redact_headers = ["authorization", "cookie"]
timeout_secs = 2
```

## JSON Extractor

Ensures the request body is parsed as JSON into `normalized_data`. This is useful when subsequent middleware expects structured JSON data but the content type might not have triggered automatic parsing, or to guarantee `normalized_data` is populated.

Config:
```toml
[middleware.ensure_json]
type = "json_extractor"
# No options required
```

## Passthru

A no-op middleware that passes requests and responses through unchanged. Useful for testing, placeholders, or temporarily disabling middleware logic without removing the configuration entry.

Config:
```toml
[middleware.noop]
type = "passthru"
```

## Policies

The policies middleware provides comprehensive policy-based access control, rate limiting, and request filtering through a flexible rule system. It supports 13 different rule types including IP filtering, path matching, geographic restrictions, rate limiting, time-based access, HTTP method filtering, User-Agent matching, Content-Type filtering, and query parameter validation.

**For complete documentation on policies and rules, see [policies-middleware.md](policies-middleware.md)**.

### Quick Example

```toml
[middleware.api_security]
type = "policies"

[[middleware.api_security.options.policies]]
id = "api_access_control"
name = "API Access Control"
enabled = true

# Allow only GET and POST methods
[[middleware.api_security.options.policies.rules]]
type = "method"
weight = 100
enabled = true
[middleware.api_security.options.policies.rules.options]
mode = "allow"
methods = ["GET", "POST"]

# Block bots and crawlers
[[middleware.api_security.options.policies.rules]]
type = "user_agent"
weight = 90
enabled = true
[middleware.api_security.options.policies.rules.options]
mode = "deny"
patterns = [
    { label = "Bots", pattern = "/bot|crawler|spider/i" }
]

# Require API key parameter
[[middleware.api_security.options.policies.rules]]
type = "query_parameter"
weight = 95
enabled = true
[middleware.api_security.options.policies.rules.options]
mode = "allow"
parameters = [
    { name = "api_key", match_type = "exists" }
]
```

**Available Rule Types**:
- IP Allow/Deny - Allowlist/blocklist by IP/CIDR
- Path - URL path filtering
- Header - HTTP header matching
- Geographic - Country-based filtering
- Rate Limit - Request throttling
- Time Based - Business hours/maintenance windows
- HTTP Method - Filter by GET/POST/etc.
- User Agent - Regex pattern matching
- Content Type - MIME type filtering
- Query Parameter - Parameter validation
- Allow All / Deny All - Control rules

See [policies-middleware.md](policies-middleware.md) for detailed documentation on all rule types, evaluation logic, configuration examples, and troubleshooting.

