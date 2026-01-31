# Extensions

**Last Updated**: 2025-01-31

Harmony provides a comprehensive extension system that allows commercial editions and custom integrations to extend the core functionality. Extensions can add new service types, middleware, authentication providers, and lifecycle hooks.

## Architecture

The extension system uses a **unified registry** where all middleware and service types (both built-in and custom) are registered as factories. This design makes built-in types and custom extensions equal citizens.

```
harmony (open-source core)
    ^
    |
harmony-commercial (private extension)
    ^
    |
harmony-aurabox (private product)
```

### Registry Flow

```
Startup
   |
   v
initialize_builtin_factories()  --> Registers all built-in services & middleware
   |
   v
HarmonyBuilder.run()
   |
   v
Register custom factories      --> Your extensions added to same registry
   |
   v
Pipeline execution             --> resolve_service() / resolve_middleware() 
                                   looks up from unified registry
```

## Quick Start

Use `HarmonyBuilder` to register custom extensions:

```rust
use harmony::{Config, HarmonyBuilder};

#[tokio::main]
async fn main() {
    let config = Config::load("config.toml").unwrap();
    
    HarmonyBuilder::new(config)
        // Register custom service factories
        .with_service_factory(Box::new(S3StorageFactory::new()))
        // Register custom middleware factories  
        .with_middleware_factory(Box::new(RateLimitFactory::new()))
        // Register custom auth providers
        .with_auth_provider(Box::new(SamlProvider::new()))
        // Register config extensions for validation
        .with_config_extension(Box::new(LicenseExtension::new()))
        // Register plugins for lifecycle hooks
        .with_plugin(Box::new(AuditPlugin::new()))
        .run()
        .await;
}
```

## Extension Types

### Service Factories

Service factories create service types that can be used as endpoints or backends. Implement `ServiceFactory` to add new service types.

#### ServiceFactory Trait

```rust
pub trait ServiceFactory: Send + Sync {
    /// Primary service name (used in config: service = "name")
    fn service_name(&self) -> &str;
    
    /// Alternative names for backward compatibility
    fn aliases(&self) -> &[&str] { &[] }
    
    /// Create a new service instance
    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>>;
    
    /// Optional description for documentation
    fn description(&self) -> Option<&str> { None }
}
```

#### Example: Custom Storage Service

```rust
use harmony::extensions::ServiceFactory;
use harmony::models::services::ServiceType;

struct S3StorageFactory;

impl ServiceFactory for S3StorageFactory {
    fn service_name(&self) -> &str {
        "s3_storage"
    }
    
    fn aliases(&self) -> &[&str] {
        &["s3", "aws_storage"]
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = serde_json::Value>> {
        Box::new(S3StorageService::new())
    }
    
    fn description(&self) -> Option<&str> {
        Some("AWS S3 compatible object storage backend")
    }
}
```

#### Configuration

Once registered, use your service in configuration:

```toml
[endpoints.my_s3_endpoint]
service = "s3_storage"  # Matches service_name() or any alias
[endpoints.my_s3_endpoint.options]
bucket = "my-bucket"
region = "us-east-1"

[backends.archive_backend]
service = "s3"  # Using alias
[backends.archive_backend.options]
bucket = "archive-bucket"
```

### Middleware Factories

Middleware factories create middleware instances that process requests and responses in the pipeline. Implement `MiddlewareFactory` to add new middleware types.

#### MiddlewareFactory Trait

```rust
pub trait MiddlewareFactory: Send + Sync {
    /// Primary type name (used in config: type = "name")
    fn type_name(&self) -> &str;
    
    /// Alternative names for backward compatibility
    fn aliases(&self) -> &[&str] { &[] }
    
    /// Create middleware instance from configuration options
    fn create(
        &self,
        options: &HashMap<String, Value>,
        config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String>;
    
    /// Validate configuration (default: tries to create instance)
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), String> {
        self.create(options, None).map(|_| ())
    }
}
```

#### Example: Rate Limiting Middleware

```rust
use harmony::extensions::MiddlewareFactory;
use harmony::models::middleware::Middleware;

struct RateLimitFactory;

impl MiddlewareFactory for RateLimitFactory {
    fn type_name(&self) -> &str {
        "rate_limit"
    }
    
    fn aliases(&self) -> &[&str] {
        &["throttle", "rate_limiter"]
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let requests_per_second = options
            .get("requests_per_second")
            .and_then(|v| v.as_u64())
            .unwrap_or(100);
            
        let burst = options
            .get("burst")
            .and_then(|v| v.as_u64())
            .unwrap_or(50);
            
        Ok(Box::new(RateLimitMiddleware::new(requests_per_second, burst)))
    }
}
```

#### Configuration

```toml
[middleware.api_rate_limit]
type = "rate_limit"  # Matches type_name() or alias
[middleware.api_rate_limit.options]
requests_per_second = 100
burst = 50
```

### Authentication Providers

Auth providers create authentication middleware. They're a specialized form of middleware factory for authentication concerns.

#### AuthProvider Trait

```rust
pub trait AuthProvider: Send + Sync {
    /// Provider name (used in config: type = "name")
    fn name(&self) -> &str;
    
    /// Validate provider configuration
    fn validate_config(&self, options: &HashMap<String, Value>) -> Result<(), String>;
    
    /// Create authentication middleware instance
    fn create_middleware(
        &self,
        options: &HashMap<String, Value>,
    ) -> Result<Box<dyn Middleware>, String>;
}
```

#### Example: SAML Provider

```rust
use harmony::extensions::AuthProvider;
use harmony::models::middleware::Middleware;

struct SamlProvider;

impl AuthProvider for SamlProvider {
    fn name(&self) -> &str {
        "saml"
    }

    fn validate_config(&self, options: &HashMap<String, Value>) -> Result<(), String> {
        if !options.contains_key("idp_url") {
            return Err("saml provider requires 'idp_url' option".to_string());
        }
        if !options.contains_key("entity_id") {
            return Err("saml provider requires 'entity_id' option".to_string());
        }
        Ok(())
    }

    fn create_middleware(
        &self,
        options: &HashMap<String, Value>,
    ) -> Result<Box<dyn Middleware>, String> {
        let idp_url = options.get("idp_url")
            .and_then(|v| v.as_str())
            .ok_or("idp_url must be a string")?;
        let entity_id = options.get("entity_id")
            .and_then(|v| v.as_str())
            .ok_or("entity_id must be a string")?;
            
        Ok(Box::new(SamlMiddleware::new(idp_url, entity_id)))
    }
}
```

#### Configuration

```toml
[middleware.saml_auth]
type = "saml"
[middleware.saml_auth.options]
idp_url = "https://idp.example.com/saml"
entity_id = "harmony-proxy"
```

### Config Extensions

Config extensions declare and validate custom configuration sections under `[extensions]`.

#### ConfigExtension Trait

```rust
pub trait ConfigExtension: Send + Sync {
    /// Extension name (matches [extensions.<name>] in config)
    fn name(&self) -> &str;
    
    /// Whether this extension's config is required
    fn is_required(&self) -> bool { false }
    
    /// Validate the extension configuration
    fn validate(&self, config: &Value) -> Result<(), String>;
    
    /// Description of expected configuration schema
    fn schema_description(&self) -> Option<&str> { None }
}
```

#### Example: License Extension

```rust
use harmony::extensions::ConfigExtension;
use serde_json::Value;

struct LicenseExtension;

impl ConfigExtension for LicenseExtension {
    fn name(&self) -> &str {
        "license"
    }

    fn is_required(&self) -> bool {
        true  // Commercial edition requires license config
    }

    fn validate(&self, config: &Value) -> Result<(), String> {
        let key = config.get("key")
            .and_then(|v| v.as_str())
            .ok_or("license.key is required")?;
        
        if key.len() < 32 {
            return Err("license.key must be at least 32 characters".to_string());
        }
        
        // Additional validation: check expiry, features, etc.
        Ok(())
    }
    
    fn schema_description(&self) -> Option<&str> {
        Some("key: string (required, min 32 chars), tier: string (optional)")
    }
}
```

#### Configuration

```toml
[extensions.license]
key = "ABCD-1234-EFGH-5678-IJKL-9012-MNOP-3456"
tier = "enterprise"
```

### Plugins (Lifecycle Hooks)

Plugins provide hooks into Harmony's lifecycle for custom behavior during startup, runtime, and shutdown.

#### HarmonyPlugin Trait

```rust
pub trait HarmonyPlugin: Send + Sync {
    /// Plugin name for logging
    fn name(&self) -> &str;
    
    /// Called after config load, before validation
    fn on_config_load(&self, config: &mut Config) -> Result<(), String> { Ok(()) }
    
    /// Called after validation, before adapters start
    fn on_pre_start(&self, config: &Config) -> Result<(), String> { Ok(()) }
    
    /// Called after all adapters have started
    fn on_post_start(&self, config: &Config, registry: &AdapterRegistry) -> Result<(), String> { Ok(()) }
    
    /// Called during graceful shutdown
    fn on_shutdown(&self) -> Result<(), String> { Ok(()) }
}
```

#### Example: Audit Plugin

```rust
use harmony::extensions::HarmonyPlugin;
use harmony::Config;
use harmony::adapters::registry::AdapterRegistry;

struct AuditPlugin {
    audit_endpoint: String,
}

impl HarmonyPlugin for AuditPlugin {
    fn name(&self) -> &str {
        "audit-logger"
    }

    fn on_pre_start(&self, config: &Config) -> Result<(), String> {
        // Log startup event
        tracing::info!(
            proxy_id = %config.proxy.id.as_deref().unwrap_or("unknown"),
            "Harmony proxy starting"
        );
        Ok(())
    }

    fn on_post_start(&self, config: &Config, registry: &AdapterRegistry) -> Result<(), String> {
        // Register with external audit system
        let networks = registry.running_networks();
        tracing::info!(
            networks = ?networks,
            "Harmony proxy started successfully"
        );
        Ok(())
    }

    fn on_shutdown(&self) -> Result<(), String> {
        // Flush audit logs, notify external systems
        tracing::info!("Harmony proxy shutting down");
        Ok(())
    }
}
```

## Built-in Factories

Harmony includes built-in factories for all standard service and middleware types. These are registered automatically when the proxy starts.

### Built-in Service Types

| Service Name | Aliases | Description |
|--------------|---------|-------------|
| `http` | - | HTTP passthrough service |
| `http3` | `h3` | HTTP/3 over QUIC service |
| `fhir` | - | FHIR R4 resource server |
| `jmix` | - | JMIX healthcare data exchange |
| `dicomweb` | - | DICOMweb QIDO-RS/WADO-RS |
| `dicom_scu` | `dicom` | DICOM SCU (outgoing DIMSE) |
| `dicom_scp` | - | DICOM SCP (incoming DIMSE listener) |
| `echo` | - | Echo service for testing |
| `storage` | - | Filesystem/S3 storage backend |
| `management` | - | Management API service |
| `mock_dicom` | - | Mock DICOM PACS for testing |
| `jmix_backend` | - | JMIX backend service |

### Built-in Middleware Types

| Type Name | Aliases | Description |
|-----------|---------|-------------|
| `jwt_auth` | `jwt` | JWT Bearer token authentication |
| `basic_auth` | - | HTTP Basic authentication |
| `connect` | - | Connection/passthrough middleware |
| `passthru` | `noop` | No-op passthrough |
| `json_extractor` | - | JSON body extraction |
| `transform` | `jolt` | JOLT JSON transformation |
| `metadata_transform` | - | Metadata field transformation |
| `path_filter` | - | URL path filtering |
| `policies` | - | Policy-based access control |
| `log_dump` | `debug`, `dump` | Request/response logging |
| `webhook` | - | External HTTP webhook |
| `mesh_auth` | - | Mesh authentication |
| `jmix_builder` | - | JMIX package builder |
| `dicomweb_bridge` | - | DICOMweb to DIMSE bridge |
| `dicom_to_dicomweb` | - | DICOM to DICOMweb conversion |
| `dicom_flatten` | - | DICOM JSON flattening |
| `dicom_unflatten` | - | DICOM JSON unflattening |

## API Reference

### UnifiedRegistry

The central registry for all factories:

```rust
use harmony::extensions::{get_registry, initialize_builtin_factories};

// Get the global registry
let registry = get_registry();

// Initialize built-in factories (called automatically at startup)
initialize_builtin_factories().await;

// Read from registry
let reg = registry.read().expect("lock");
if let Some(factory) = reg.find_service_factory("http") {
    let service = factory.create();
}
```

### HarmonyBuilder

Fluent builder for configuring and starting Harmony:

```rust
use harmony::{Config, HarmonyBuilder};

let builder = HarmonyBuilder::new(config)
    .with_config_path(PathBuf::from("config.toml"))
    .with_service_factory(Box::new(MyServiceFactory))
    .with_middleware_factory(Box::new(MyMiddlewareFactory))
    .with_auth_provider(Box::new(MyAuthProvider))
    .with_config_extension(Box::new(MyConfigExtension))
    .with_plugin(Box::new(MyPlugin));

// Run the proxy (blocks until shutdown)
builder.run().await;
```

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

## See Also

- [middleware.md](middleware.md) - Built-in middleware reference
- [endpoints.md](endpoints.md) - Endpoint configuration
- [backends.md](backends.md) - Backend configuration
- [adapters.md](adapters.md) - Protocol adapter implementation guide
- [configuration.md](configuration.md) - General configuration reference
