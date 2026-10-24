---
title: Implementing Extensions
description: How to implement custom services and middleware as extensions.
status: active
updated: 2026-10-24
type: Architecture
related:
  - architecture/extensions.md
---

# Implementing Extensions

How to implement custom services and middleware as extensions. This page is part of [Extensions](extensions.md).

## Implementing Custom Services

To implement a custom service type, you need to implement both `ServiceType` and `ServiceHandler`:

```rust
use harmony::models::services::services::{ServiceType, ServiceHandler};
use harmony::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use harmony::config::config::ConfigError;
use harmony::router::route_config::RouteConfig;
use harmony::utils::Error;
use async_trait::async_trait;
use std::collections::HashMap;
use serde_json::Value;
use axum::response::Response;

struct MyCustomService;

#[async_trait]
impl ServiceType for MyCustomService {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate service configuration
        if options.get("required_option").is_none() {
            return Err(ConfigError::MissingField("required_option".to_string()));
        }
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Define routes this service handles
        vec![
            RouteConfig::new("/api/custom/*path", vec!["GET", "POST"]),
        ]
    }
}

#[async_trait]
impl ServiceHandler<Value> for MyCustomService {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Process incoming request for endpoint
        Ok(envelope)
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Convert response envelope to HTTP response
        Ok(axum::response::Response::builder()
            .status(envelope.response_details.status)
            .body(envelope.original_data.into())
            .unwrap())
    }

    async fn backend_outgoing_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // Make outgoing request to backend target
        // ... implement backend logic ...
        Ok(ResponseEnvelope::from_backend(
            envelope.request_details,
            200,
            HashMap::new(),
            vec![],
            None,
        ))
    }
}
```

## Implementing Custom Middleware

To implement custom middleware, implement the `Middleware` trait:

```rust
use harmony::models::middleware::middleware::Middleware;
use harmony::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use harmony::config::config::ConfigError;
use harmony::utils::Error;
use async_trait::async_trait;
use std::collections::HashMap;
use serde_json::Value;

struct MyCustomMiddleware {
    threshold: u64,
}

#[async_trait]
impl Middleware for MyCustomMiddleware {
    fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        Ok(())
    }

    async fn left(
        &self,
        mut envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<Value>, Error> {
        // Process incoming request (before backend)
        envelope.request_details.metadata.insert(
            "processed_by".to_string(),
            "my_custom_middleware".to_string(),
        );
        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<Value>,
    ) -> Result<ResponseEnvelope<Value>, Error> {
        // Process outgoing response (after backend)
        Ok(envelope)
    }
}
```

## Best Practices

### 1. Use Aliases for Backward Compatibility

When renaming service or middleware types, keep old names as aliases:

```rust
fn aliases(&self) -> &[&str] {
    &["old_name", "legacy_name"]
}
```

### 2. Validate Early

Implement thorough validation in `validate()` methods to catch configuration errors at startup rather than runtime.

### 3. Use Config Extensions for Complex Configuration

For extensions with multiple configuration options, use `ConfigExtension` to provide structured validation and documentation.

### 4. Leverage Lifecycle Hooks

Use `HarmonyPlugin` hooks for:
- License validation (`on_pre_start`)
- Metrics initialization (`on_post_start`)
- Graceful cleanup (`on_shutdown`)

### 5. Thread Safety

All extension traits require `Send + Sync`. Ensure your implementations are thread-safe.
