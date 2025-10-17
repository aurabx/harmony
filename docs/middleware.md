# Middleware

Middleware extends the request/response pipeline to authenticate, enrich, or transform the Envelope as it flows between endpoints and backends.

- Authentication middleware runs at the start of the pipeline (endpoint side)
- Transformation middleware can run before requests are sent to backends and/or on responses

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

## Path Filter

Filters incoming requests based on URL path patterns using matchit syntax. Requests that don't match any configured rule are rejected with HTTP 404 and backend processing is skipped.

Config keys:
- `rules` (array of strings, required): List of path patterns to allow using matchit syntax (e.g., "/ImagingStudy", "/Patient/{id}")

Example:
```toml
[middleware.imagingstudy_filter]
type = "path_filter"
[middleware.imagingstudy_filter.options]
rules = ["/ImagingStudy", "/Patient"]
```

Behavior:
- Only applies to incoming requests (left side of middleware chain)
- Path matching uses the subpath after the endpoint's path_prefix
- Trailing slashes are normalized (e.g., "/ImagingStudy/" matches "/ImagingStudy")
- On rejection: returns 404 status with empty body and sets skip_backends=true to avoid backend calls
- Supports matchit patterns for dynamic routing (wildcards, parameters)

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

