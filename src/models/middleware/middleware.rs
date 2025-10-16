use crate::config::config::{Config, ConfigError};
use crate::models::envelope::envelope::RequestEnvelope;
use crate::utils::Error;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct MiddlewareInstance {
    pub middleware_type: String, // The middleware type, e.g., "jwtauth", "auth", etc.
    #[serde(default)]
    pub options: Option<HashMap<String, serde_json::Value>>, // Middleware-specific options
}

// Create a static empty HashMap to avoid the temporary value issue
static EMPTY_OPTIONS: Lazy<HashMap<String, serde_json::Value>> = Lazy::new(HashMap::new);

impl MiddlewareInstance {
    /// Resolves the middleware type using the centralized middleware resolver
    pub fn resolve_middleware(&self) -> Result<Box<dyn Middleware>, String> {
        let options = self.options.as_ref().unwrap_or(&EMPTY_OPTIONS);
        resolve_middleware(&self.middleware_type, options)
    }
}

// Middleware registry similar to services
pub static MIDDLEWARE_REGISTRY: OnceCell<HashMap<String, String>> = OnceCell::new();

#[derive(Debug, serde::Deserialize, Default, Clone)]
#[serde(default)]
pub struct MiddlewareConfig {
    pub module: String, // Path to the module or metadata
}

pub fn initialise_middleware_registry(config: &Config) {
    // Populate the registry using middleware types from the provided config
    let registry = config
        .middleware_types
        .iter()
        .map(|(key, value)| (key.clone(), value.module.clone()))
        .collect();

    // Set the MIDDLEWARE_REGISTRY value; this will panic if called more than once
    MIDDLEWARE_REGISTRY
        .set(registry)
        .expect("MIDDLEWARE_REGISTRY can only be initialized once");
}

/// Resolves a middleware type from the registry and returns a boxed Middleware
pub fn resolve_middleware(
    middleware_type: &str,
    options: &HashMap<String, Value>,
) -> Result<Box<dyn Middleware>, String> {
    // Check the registry first
    if let Some(registry) = MIDDLEWARE_REGISTRY.get() {
        if let Some(module) = registry.get(middleware_type) {
            match module.as_str() {
                "" => {
                    // Default built-in modules
                    create_builtin_middleware(middleware_type, options)
                }
                module_path => {
                    // Custom module loading would go here
                    Err(format!(
                        "Middleware type '{}' references module '{}' but dynamic loading is not implemented yet",
                        middleware_type, module_path
                    ))
                }
            }
        } else {
            // Registry is present but does not include this middleware. Attempt built-in fallback.
            match create_builtin_middleware(middleware_type, options) {
                Ok(mw) => Ok(mw),
                Err(_) => Err(format!("Unknown middleware type: {}", middleware_type)),
            }
        }
    } else {
        // Fallback to hardcoded types if registry isn't initialized
        create_builtin_middleware(middleware_type, options)
    }
}

/// Creates built-in middleware instances
fn create_builtin_middleware(
    middleware_type: &str,
    options: &HashMap<String, Value>,
) -> Result<Box<dyn Middleware>, String> {
    use crate::models::middleware::types::auth::AuthSidecarMiddleware;
    use crate::models::middleware::types::connect::AuraboxConnectMiddleware;
    use crate::models::middleware::types::jwtauth::JwtAuthMiddleware;
    use crate::models::middleware::types::transform::JoltTransformMiddleware;

    match middleware_type.to_lowercase().as_str() {
        "jwtauth" => {
            let config = crate::models::middleware::types::jwtauth::parse_config(options)?;
            Ok(Box::new(JwtAuthMiddleware::new(config)))
        }
        "auth" => {
            let config = crate::models::middleware::types::auth::parse_config(options)?;
            Ok(Box::new(AuthSidecarMiddleware::new(config)))
        }
        "connect" => {
            let config = crate::models::middleware::types::connect::parse_config(options)?;
            Ok(Box::new(AuraboxConnectMiddleware::new(config)))
        }
        "passthru" => Ok(Box::new(
            crate::models::middleware::types::passthru::PassthruMiddleware::new(),
        )),
        "json_extractor" | "json" => Ok(Box::new(
            crate::models::middleware::types::json_extractor::JsonExtractorMiddleware::new(),
        )),
        "jmix_builder" => Ok(Box::new(
            crate::models::middleware::types::jmix_builder::JmixBuilderMiddleware::new(),
        )),
        "dicomweb_bridge" | "dicomweb" => Ok(Box::new(
            crate::models::middleware::types::dicomweb_bridge::DicomwebBridgeMiddleware::new(),
        )),
        "transform" => {
            let config = crate::models::middleware::types::transform::parse_config(options)?;
            Ok(Box::new(JoltTransformMiddleware::new(config)?))
        }
        _ => Err(format!(
            "Unsupported built-in middleware type: {}",
            middleware_type
        )),
    }
}

#[async_trait]
pub trait Middleware: Send + Sync {
    /// Validate the middleware configuration
    fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Default implementation - can be overridden
        Ok(())
    }

    /// Modify the outgoing envelope on its way to the backend.
    async fn left(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error>;

    /// Modify the incoming envelope on its way from the backend.
    async fn right(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error>;
}
