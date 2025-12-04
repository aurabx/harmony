# Pipeline Middleware Split Configuration

As of v0.9.0, Harmony supports two ways to configure middleware in pipelines:
1. **List format**: Simple array where middleware applies to both request and response paths
2. **Split format**: Separate left and right chains for request-specific and response-specific middleware

## Overview

In a Harmony pipeline, middleware is executed in two phases:
- **Left chain (request)**: Middleware executes in order as the request flows to the backend
- **Right chain (response)**: Middleware executes in reverse order as the response returns from the backend

Previously, both chains used the same middleware list. Now you can configure them independently.

## List Format (Backward Compatible)

The traditional format where middleware applies to both request and response paths:

```toml
[pipelines.my_pipeline]
description = "My pipeline"
networks = ["default"]
endpoints = ["ep"]
backends = ["be"]
middleware = ["auth", "validate", "transform"]
```

This is equivalent to:
```toml
[pipelines.my_pipeline.middleware]
left = ["auth", "validate", "transform"]
right = ["auth", "validate", "transform"]
```

The right chain executes in reverse order (transform, validate, auth) due to how the middleware chain processes responses.

## Split Format (New)

Configure different middleware for requests and responses:

```toml
[pipelines.my_pipeline]
description = "My pipeline"
networks = ["default"]
endpoints = ["ep"]
backends = ["be"]

[pipelines.my_pipeline.middleware]
left = ["auth", "validate", "transform"]    # Request path only
right = ["log", "encrypt"]                  # Response path only
```

### Left-Only Middleware

Process middleware only on the request path (before the backend):

```toml
[pipelines.my_pipeline.middleware]
left = ["auth", "validate", "sanitize"]
```

When the response returns, the right chain is empty (no middleware processing).

### Right-Only Middleware

Process middleware only on the response path (after the backend):

```toml
[pipelines.my_pipeline.middleware]
right = ["log", "transform", "encrypt"]
```

When the request is processed, the left chain is empty (no middleware processing).

## Execution Order

### Left Chain (Request to Backend)
Middleware executes in order:
```
request → [1] → [2] → [3] → backend
```

### Right Chain (Response from Backend)
Middleware executes in **reverse** order:
```
backend → [3] → [2] → [1] → response
```

### Example: Symmetric Middleware
```toml
[pipelines.my_pipeline.middleware]
left = ["auth", "log"]      # auth → log → backend
right = ["log", "auth"]     # backend → auth → log → response
```

This ensures `auth` processes the request first and the response last, with `log` in the middle on both paths.

## Use Cases

### 1. Request Validation Only
```toml
[pipelines.api_gateway.middleware]
left = ["jwt_auth", "rate_limit", "validate"]
# No right chain needed for read-only GET operations
```

### 2. Response Transformation and Logging
```toml
[pipelines.data_export.middleware]
left = ["auth"]                              # Only authenticate requests
right = ["transform_format", "compress", "log"]  # Transform and log responses
```

### 3. Sensitive Data Protection
```toml
[pipelines.pii_endpoint.middleware]
left = ["auth", "pii_check"]        # Validate request doesn't expose PII
right = ["redact_pii", "encrypt"]   # Redact response before sending
```

### 4. Asymmetric Request/Response Paths
```toml
[pipelines.webhook.middleware]
left = ["sign_request"]             # Sign outgoing webhook payload
right = ["verify_signature"]        # Verify response signature
```

## Configuration Validation

- Both list and split formats cannot be mixed in the same pipeline
- Empty middleware lists (both left and right empty) are valid but logged as a warning
- All middleware names must be defined in the configuration
- Unknown middleware names cause immediate validation failure

## The "apply" Key Behavior

Some middleware types support an "apply" key in their options to control whether they execute on the request path ("left"), response path ("right"), or both ("both"). Examples include:
- `transform`: JOLT transformations
- `metadata_transform`: Metadata transformations
- `log_dump`: Logging middleware

**When using split pipeline middleware configuration**, the "apply" key is **ignored**:

```toml
# Using list format - apply key controls direction
[pipelines.simple]
middleware = ["my_transform"]  # Will use apply value from middleware.my_transform options

[middleware.my_transform]
middleware_type = "transform"
[middleware.my_transform.options]
spec_path = "spec.json"
apply = "left"  # This controls execution direction

# Using split format - apply key is ignored
[pipelines.split_pipeline]

[pipelines.split_pipeline.middleware]
left = ["my_transform"]   # Always applies only to requests
right = ["other_transform"]  # Always applies only to responses

[middleware.my_transform]
middleware_type = "transform"
[middleware.my_transform.options]
spec_path = "spec.json"
apply = "both"  # IGNORED - middleware only runs on left chain in split config
```

This makes split configuration clearer: you explicitly specify the execution path rather than relying on the "apply" configuration key.

## API Endpoint Response

The `/admin/pipelines` management endpoint now returns separate `middleware_left` and `middleware_right` arrays:

```json
{
  "pipelines": [
    {
      "id": "my_pipeline",
      "description": "My pipeline",
      "networks": ["default"],
      "endpoints": ["ep"],
      "backends": ["be"],
      "middleware_left": ["auth", "validate"],
      "middleware_right": ["log"]
    }
  ]
}
```

## Migration Guide

No migration required! Existing pipelines using the list format continue to work unchanged:

### Before (Still Supported)
```toml
[pipelines.my_pipeline]
middleware = ["auth", "validate", "transform"]
```

### After (Optionally Use Split Format)
```toml
[pipelines.my_pipeline.middleware]
left = ["auth", "validate", "transform"]
right = ["log"]
```

## Examples

See `examples/middleware-split/config.toml` for complete working examples of:
- Simple list-format pipelines
- Split-format pipelines with different left/right chains
- Left-only middleware pipelines
- Right-only middleware pipelines
