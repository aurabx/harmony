---
title: Middleware
description: Reference for authentication, transform, path filter, and other built-in middleware.
status: active
updated: 2026-10-24
type: Configuration
---

# Middleware

Middleware extends the request/response pipeline to authenticate, enrich, or transform the `RequestEnvelope` and `ResponseEnvelope` as they flow through the `PipelineExecutor`.

> **Custom Middleware**: To create your own middleware types, see [Extension Types](../architecture/extensions-types.md#middleware-factories).

## Architecture

Middleware operates within the unified `PipelineExecutor` (see [router.md](../architecture/router.md)) and processes envelopes for all protocols (HTTP, DIMSE, HL7, etc.).

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
- **Authentication unavailable** (the mesh JWKS source is unreachable, returns an error status, or serves a document that cannot be parsed): HTTP 503 Service Unavailable. The token may be valid, so the client should retry. A token whose key ID names a key the proxy cannot use is an authentication failure (401).
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
- Startup safety: config validation rejects the instance with `InvalidMiddleware` when the key material is unusable. This covers RS256 with a missing or invalid `public_key_path`, and HS256 with a missing or empty `hs256_secret`. There is no default secret.

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

## Transform Middleware

Reference for middleware that transforms or converts request and response data. See [Transform Middleware](middleware-transforms.md).

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

## Utility Middleware

Reference for the log dump, webhook, and passthru middleware. See [Utility Middleware](middleware-utilities.md).

## Policies

The policies middleware admits or rejects requests using rules grouped into policies. It has 13 rule types, including IP allow and deny lists, rate limiting, path, HTTP method, User-Agent, Content-Type, and time-based rules. By default, header, query parameter, and geographic rules do not see HTTP request data.

**For complete documentation on policies and rules, see [policies-middleware.md](policies-middleware.md)**.

### Quick Example

The middleware lists policy ids. Policies and rules are top-level `[policies.<id>]` and `[rules.<id>]` tables.

```toml
[middleware.api_security]
type = "policies"
[middleware.api_security.options]
policies = ["api_access_control"]

[policies.api_access_control]
id = "api_access_control"
name = "API Access Control"
rules = ["get_post_only", "block_bots"]

# Allow only GET and POST. Other methods get 405.
[rules.get_post_only]
id = "get_post_only"
type = "method"
weight = 100
[rules.get_post_only.options]
mode = "allow"
methods = ["GET", "POST"]

# Block bots and crawlers with 403
[rules.block_bots]
id = "block_bots"
type = "user_agent"
weight = 90
[rules.block_bots.options]
mode = "deny"
patterns = [
    { label = "Bots", pattern = "/bot|crawler|spider/i" }
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

See [policies-middleware.md](policies-middleware.md) for detailed documentation on all rule types, evaluation logic, configuration examples, known issues, and troubleshooting.

