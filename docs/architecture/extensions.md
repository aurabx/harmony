---
title: Extensions
description: The extension system for registering custom services, middleware, and plugins.
status: active
updated: 2026-10-24
type: Architecture
---

# Extensions

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

Reference for each kind of extension Harmony supports. See [Extension Types](extensions-types.md).

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

## Implementing Extensions

How to implement custom services and middleware as extensions. See [Implementing Extensions](extensions-implementing.md).

## See Also

- [middleware.md](../configuration/middleware.md) - Built-in middleware reference
- [endpoints.md](../configuration/endpoints.md) - Endpoint configuration
- [backends.md](../configuration/backends.md) - Backend configuration
- [adapters.md](adapters.md) - Protocol adapter implementation guide
- [Configuration](../configuration/configuration-file.md) - General configuration reference
