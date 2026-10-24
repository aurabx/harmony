---
title: Extension Types
description: Reference for each kind of extension Harmony supports.
status: active
updated: 2026-10-24
type: Architecture
related:
  - architecture/extensions.md
---

# Extension Types

Reference for each kind of extension Harmony supports. This page is part of [Extensions](extensions.md).

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

    /// Called by config validation at load and reload for every
    /// middleware instance of this type (default: builds and discards it)
    fn validate_config(
        &self,
        options: &HashMap<String, Value>,
        config: &Config,
    ) -> Result<(), String> {
        self.create(options, Some(config)).map(|_| ())
    }
}
```

Config validation calls `validate_config`, not `validate`. Override `validate_config` when `create()` has side effects, such as spawning tasks or writing to global state, or when it can panic. Run only the parsing part.

Factories registered through `HarmonyBuilder` are added after the first config load. Their instances are not checked at startup. They are checked on every reload. A bad instance can therefore start, then cause the next reload to be rejected.

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
