//! Unified registry for middleware and service factories.
//!
//! This module provides a single registry that holds both built-in and custom
//! factories for middleware and services. All types are registered through
//! the same mechanism, making built-ins and custom extensions equal citizens.

use crate::config::config::Config;
use crate::extensions::auth::AuthProvider;
use crate::extensions::config_extension::ConfigExtension;
use crate::extensions::middleware::MiddlewareFactory;
use crate::extensions::service::ServiceFactory;
use crate::models::middleware::middleware::Middleware;
use crate::models::services::services::ServiceType;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Unified registry for all factories and providers.
///
/// This registry holds:
/// - Middleware factories (both built-in and custom)
/// - Service factories (both built-in and custom)
/// - Auth providers (custom only, create middleware)
/// - Config extensions (for validating extension config)
pub struct UnifiedRegistry {
    middleware_factories: HashMap<String, Box<dyn MiddlewareFactory>>,
    service_factories: HashMap<String, Box<dyn ServiceFactory>>,
    auth_providers: HashMap<String, Box<dyn AuthProvider>>,
    config_extensions: HashMap<String, Box<dyn ConfigExtension>>,
}

impl UnifiedRegistry {
    /// Creates a new empty registry
    pub fn new() -> Self {
        Self {
            middleware_factories: HashMap::new(),
            service_factories: HashMap::new(),
            auth_providers: HashMap::new(),
            config_extensions: HashMap::new(),
        }
    }

    /// Register a middleware factory
    ///
    /// Registers the factory under its primary name and all aliases.
    /// All names are stored in lowercase for case-insensitive lookup.
    pub fn register_middleware_factory(&mut self, factory: Box<dyn MiddlewareFactory>) {
        let name = factory.type_name().to_lowercase();
        let aliases: Vec<String> = factory.aliases().iter().map(|a| a.to_lowercase()).collect();

        tracing::debug!(
            "Registering middleware factory: {} (aliases: {:?})",
            name,
            aliases
        );

        // Store factory under primary name
        // For aliases, we need to clone the factory reference - but we can't clone Box<dyn>
        // Instead, we store the primary name and look up by alias in find_middleware_factory
        self.middleware_factories.insert(name, factory);
    }

    /// Register a service factory
    ///
    /// Registers the factory under its primary name.
    /// All names are stored in lowercase for case-insensitive lookup.
    pub fn register_service_factory(&mut self, factory: Box<dyn ServiceFactory>) {
        let name = factory.service_name().to_lowercase();
        let aliases: Vec<String> = factory.aliases().iter().map(|a| a.to_lowercase()).collect();

        tracing::debug!(
            "Registering service factory: {} (aliases: {:?})",
            name,
            aliases
        );

        self.service_factories.insert(name, factory);
    }

    /// Register an auth provider
    pub fn register_auth_provider(&mut self, provider: Box<dyn AuthProvider>) {
        let name = provider.name().to_string();
        tracing::debug!("Registering auth provider: {}", name);
        self.auth_providers.insert(name, provider);
    }

    /// Register a config extension
    pub fn register_config_extension(&mut self, extension: Box<dyn ConfigExtension>) {
        let name = extension.name().to_string();
        tracing::debug!("Registering config extension: {}", name);
        self.config_extensions.insert(name, extension);
    }

    /// Find a middleware factory by type name or alias
    pub fn find_middleware_factory(&self, type_name: &str) -> Option<&dyn MiddlewareFactory> {
        let lower = type_name.to_lowercase();

        // First try direct lookup
        if let Some(factory) = self.middleware_factories.get(&lower) {
            return Some(factory.as_ref());
        }

        // Then check aliases
        for factory in self.middleware_factories.values() {
            if factory.aliases().iter().any(|a| a.to_lowercase() == lower) {
                return Some(factory.as_ref());
            }
        }

        None
    }

    /// Find a service factory by name or alias
    pub fn find_service_factory(&self, name: &str) -> Option<&dyn ServiceFactory> {
        let lower = name.to_lowercase();

        // First try direct lookup
        if let Some(factory) = self.service_factories.get(&lower) {
            return Some(factory.as_ref());
        }

        // Then check aliases
        for factory in self.service_factories.values() {
            if factory.aliases().iter().any(|a| a.to_lowercase() == lower) {
                return Some(factory.as_ref());
            }
        }

        None
    }

    /// Find an auth provider by name
    pub fn find_auth_provider(&self, name: &str) -> Option<&dyn AuthProvider> {
        self.auth_providers.get(name).map(|p| p.as_ref())
    }

    /// Find a config extension by name
    pub fn find_config_extension(&self, name: &str) -> Option<&dyn ConfigExtension> {
        self.config_extensions.get(name).map(|e| e.as_ref())
    }

    /// Get all registered middleware type names
    pub fn middleware_type_names(&self) -> Vec<&str> {
        self.middleware_factories.keys().map(|s| s.as_str()).collect()
    }

    /// Get all registered service names
    pub fn service_names(&self) -> Vec<&str> {
        self.service_factories.keys().map(|s| s.as_str()).collect()
    }

    /// Get all registered config extension names
    pub fn config_extension_names(&self) -> Vec<&str> {
        self.config_extensions.keys().map(|s| s.as_str()).collect()
    }

    /// Resolve a middleware type and create an instance
    pub fn resolve_middleware(
        &self,
        type_name: &str,
        options: &HashMap<String, Value>,
        config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        // First check middleware factories
        if let Some(factory) = self.find_middleware_factory(type_name) {
            return factory.create(options, config);
        }

        // Then check auth providers (they create middleware too)
        if let Some(provider) = self.find_auth_provider(type_name) {
            return provider.create_middleware(options);
        }

        Err(format!("Unknown middleware type: {}", type_name))
    }

    /// Resolve a service type and create an instance
    pub fn resolve_service(
        &self,
        service_name: &str,
    ) -> Result<Box<dyn ServiceType<ReqBody = Value>>, String> {
        if let Some(factory) = self.find_service_factory(service_name) {
            return Ok(factory.create());
        }

        Err(format!("Unknown service type: {}", service_name))
    }

    /// Check if the registry has been initialized with factories
    pub fn is_initialized(&self) -> bool {
        !self.middleware_factories.is_empty() || !self.service_factories.is_empty()
    }

    /// Validate all config extensions against the provided config
    pub fn validate_config_extensions(&self, config: &Config) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        for (name, extension) in &self.config_extensions {
            if let Some(ext_config) = config.extensions.get(name) {
                if let Err(e) = extension.validate(ext_config) {
                    errors.push(format!("Extension '{}': {}", name, e));
                }
            } else if extension.is_required() {
                errors.push(format!(
                    "Extension '{}' is required but not configured",
                    name
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Default for UnifiedRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global unified registry
static UNIFIED_REGISTRY: std::sync::OnceLock<Arc<RwLock<UnifiedRegistry>>> =
    std::sync::OnceLock::new();

/// Get the global unified registry
pub fn get_registry() -> Arc<RwLock<UnifiedRegistry>> {
    UNIFIED_REGISTRY
        .get_or_init(|| Arc::new(RwLock::new(UnifiedRegistry::new())))
        .clone()
}

/// Initialize the registry with built-in factories.
/// This should be called once at startup before any middleware/service resolution.
/// Now uses std::sync::RwLock so this is synchronous but still async-compatible.
pub async fn initialize_builtin_factories() {
    initialize_builtin_factories_sync();
}

/// Synchronously initialize the registry with built-in factories.
/// Thread-safe and idempotent - safe to call multiple times from any thread.
/// Uses std::sync::Once internally to ensure single initialization.
pub fn initialize_builtin_factories_sync() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        let registry = get_registry();
        let mut reg = registry
            .write()
            .expect("Failed to acquire write lock on unified registry");

        // Register built-in middleware factories
        register_builtin_middleware(&mut reg);

        // Register built-in service factories
        register_builtin_services(&mut reg);

        tracing::info!(
            "Initialized unified registry with {} middleware types and {} service types",
            reg.middleware_type_names().len(),
            reg.service_names().len()
        );
    });
}

/// Register all built-in middleware factories
fn register_builtin_middleware(registry: &mut UnifiedRegistry) {
    use crate::extensions::builtin::middleware::*;

    // Auth middleware
    registry.register_middleware_factory(Box::new(JwtAuthFactory));
    registry.register_middleware_factory(Box::new(BasicAuthFactory));

    // Core middleware
    registry.register_middleware_factory(Box::new(ConnectFactory));
    registry.register_middleware_factory(Box::new(PassthruFactory));
    registry.register_middleware_factory(Box::new(JsonExtractorFactory));

    // DICOM middleware
    registry.register_middleware_factory(Box::new(JmixBuilderFactory));
    registry.register_middleware_factory(Box::new(DicomwebBridgeFactory));
    registry.register_middleware_factory(Box::new(DicomToDicomwebFactory));
    registry.register_middleware_factory(Box::new(DicomFlattenFactory));
    registry.register_middleware_factory(Box::new(DicomUnflattenFactory));

    // Transform middleware
    registry.register_middleware_factory(Box::new(TransformFactory));
    registry.register_middleware_factory(Box::new(MetadataTransformFactory));

    // Filter/policy middleware
    registry.register_middleware_factory(Box::new(PathFilterFactory));
    registry.register_middleware_factory(Box::new(PoliciesFactory));

    // Utility middleware
    registry.register_middleware_factory(Box::new(LogDumpFactory));
    registry.register_middleware_factory(Box::new(WebhookFactory));
    registry.register_middleware_factory(Box::new(MeshAuthFactory));
}

/// Register all built-in service factories
fn register_builtin_services(registry: &mut UnifiedRegistry) {
    use crate::extensions::builtin::services::*;

    // HTTP services
    registry.register_service_factory(Box::new(HttpServiceFactory));
    registry.register_service_factory(Box::new(Http3ServiceFactory));
    registry.register_service_factory(Box::new(EchoServiceFactory));

    // FHIR services
    registry.register_service_factory(Box::new(FhirServiceFactory));

    // DICOM services
    registry.register_service_factory(Box::new(DicomScuServiceFactory));
    registry.register_service_factory(Box::new(DicomScpServiceFactory));
    registry.register_service_factory(Box::new(DicomwebServiceFactory));
    registry.register_service_factory(Box::new(MockDicomServiceFactory));

    // Jmix services
    registry.register_service_factory(Box::new(JmixServiceFactory));
    registry.register_service_factory(Box::new(JmixBackendServiceFactory));

    // Storage services
    registry.register_service_factory(Box::new(StorageServiceFactory));

    // Management services
    registry.register_service_factory(Box::new(ManagementServiceFactory));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = UnifiedRegistry::new();
        assert!(registry.middleware_type_names().is_empty());
        assert!(registry.service_names().is_empty());
    }

    #[test]
    fn test_find_nonexistent() {
        let registry = UnifiedRegistry::new();
        assert!(registry.find_middleware_factory("nonexistent").is_none());
        assert!(registry.find_service_factory("nonexistent").is_none());
        assert!(registry.find_auth_provider("nonexistent").is_none());
    }
}
