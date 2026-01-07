# Adding Custom Middleware to Harmony

## Reference Documentation
Full details: `docs/middleware.md`

## Overview

Middleware processes `RequestEnvelope` and `ResponseEnvelope` as they flow through the pipeline. Middleware is protocol-agnostic—it works with envelopes, not raw protocol data.

```
RequestEnvelope → Incoming Middleware → Backend → Outgoing Middleware → ResponseEnvelope
```

## Files to Modify

1. **Middleware implementation**: `src/pipeline/middleware/<your_middleware>.rs`
2. **Middleware type registration**: `src/pipeline/middleware/mod.rs`
3. **Config parsing**: Add to middleware type enum and parsing
4. **Tests**: `tests/` - integration tests for the middleware

## Step-by-Step

### 1. Create the Middleware Module

In `src/pipeline/middleware/your_middleware.rs`:

```rust
use crate::models::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::pipeline::middleware::{Middleware, MiddlewareError, MiddlewareResult};
use async_trait::async_trait;

pub struct YourMiddleware {
    // Configuration fields
    option_a: String,
    option_b: bool,
}

impl YourMiddleware {
    pub fn new(option_a: String, option_b: bool) -> Self {
        Self { option_a, option_b }
    }
}

#[async_trait]
impl Middleware for YourMiddleware {
    /// Process incoming request (before backend)
    async fn process_request(
        &self,
        envelope: RequestEnvelope,
    ) -> MiddlewareResult<RequestEnvelope> {
        // Modify or validate the request envelope
        // Return Err(MiddlewareError::...) to reject
        Ok(envelope)
    }

    /// Process outgoing response (after backend)
    async fn process_response(
        &self,
        envelope: ResponseEnvelope,
    ) -> MiddlewareResult<ResponseEnvelope> {
        // Modify the response envelope
        Ok(envelope)
    }
}
```

### 2. Register in mod.rs

In `src/pipeline/middleware/mod.rs`:

```rust
pub mod your_middleware;
pub use your_middleware::YourMiddleware;
```

### 3. Add Configuration Parsing

The middleware needs to be constructible from TOML config. Add to the middleware factory/builder.

### 4. Error Handling

Use appropriate `MiddlewareError` variants:
- `MiddlewareError::Authentication` → HTTP 401
- `MiddlewareError::PathDenied` → HTTP 404  
- Other errors → HTTP 500

## Configuration Example

```toml
[middleware.your_middleware_instance]
type = "your_middleware"
[middleware.your_middleware_instance.options]
option_a = "value"
option_b = true
```

## Common Patterns

### Request-only middleware (e.g., auth)
```rust
async fn process_request(&self, envelope: RequestEnvelope) -> MiddlewareResult<RequestEnvelope> {
    // Validate and return Ok or Err
}

async fn process_response(&self, envelope: ResponseEnvelope) -> MiddlewareResult<ResponseEnvelope> {
    Ok(envelope) // Pass through unchanged
}
```

### Response-only middleware (e.g., response transform)
```rust
async fn process_request(&self, envelope: RequestEnvelope) -> MiddlewareResult<RequestEnvelope> {
    Ok(envelope) // Pass through unchanged
}

async fn process_response(&self, envelope: ResponseEnvelope) -> MiddlewareResult<ResponseEnvelope> {
    // Transform response
}
```

### Accessing headers/metadata
```rust
let auth_header = envelope.request_details.headers.get("authorization");
let path = &envelope.request_details.path;
```

## Testing

```rust
#[tokio::test]
async fn test_your_middleware_allows_valid_request() {
    let middleware = YourMiddleware::new("value".into(), true);
    let envelope = create_test_envelope();
    
    let result = middleware.process_request(envelope).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_your_middleware_rejects_invalid_request() {
    let middleware = YourMiddleware::new("value".into(), true);
    let envelope = create_invalid_envelope();
    
    let result = middleware.process_request(envelope).await;
    assert!(matches!(result, Err(MiddlewareError::Authentication(_))));
}
```
