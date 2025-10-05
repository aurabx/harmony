# Middleware

Middleware extends the request/response pipeline to authenticate, enrich, or transform the Envelope as it flows between endpoints and backends.

- Authentication middleware runs at the start of the pipeline (endpoint side)
- Transformation middleware can run before requests are sent to backends and/or on responses

## Authentication

### Basic Auth
Validates a `username`/`password` combination, typically supplied in the `Authorization: Basic <base64>` header.

Config keys:
- `username` (string)
- `password` (string)
- `token_path` (optional, string): file path for a pre-shared token, if used by your environment

Example:
```toml
[middleware.basic_auth]
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
[middleware.jwt_auth]
public_key_path = "/etc/harmony/jwt_public.pem"
issuer = "https://auth.example.com/"
audience = "harmony"
leeway_secs = 60
```

- HS256 (development/test only):
```toml
[middleware.jwt_auth]
use_hs256 = true
hs256_secret = "replace-with-strong-secret"
issuer = "https://auth.example.com/"
audience = "harmony"
leeway_secs = 60
```

Notes:
- Place JWT auth middleware early in your pipeline to reject unauthenticated requests before expensive work.
- Configuration parsing for this middleware lives within the middleware module itself.

## Transformation

### DICOM to JSON
Converts DICOM into JSON.

### DICOMweb to JSON
Converts DICOMweb into JSON.

### FHIR to DICOM
Converts FHIR to DICOM.

### JMIX to DICOM
Converts JMIX to DICOM.
