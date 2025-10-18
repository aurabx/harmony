use crate::config::config::{Config, ConfigError};
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::utils::Error;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::Value;
use std::collections::HashMap;

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
pub fn resolve_middleware_type(
    middleware_type: &str,
    options: &HashMap<String, Value>,
) -> Result<Box<dyn Middleware>, String> {
    // Check the registry first
    if let Some(registry) = MIDDLEWARE_REGISTRY.get() {
        if let Some(module) = registry.get(middleware_type) {
            match module.as_str() {
                "" => {
                    // Default built-in modules
                    create_builtin_middleware_type(middleware_type, options)
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
            match create_builtin_middleware_type(middleware_type, options) {
                Ok(mw) => Ok(mw),
                Err(_) => Err(format!("Unknown middleware type: {}", middleware_type)),
            }
        }
    } else {
        // Fallback to hardcoded types if registry isn't initialized
        create_builtin_middleware_type(middleware_type, options)
    }
}

/// Creates built-in middleware instances
fn create_builtin_middleware_type(
    middleware_type: &str,
    options: &HashMap<String, Value>,
) -> Result<Box<dyn Middleware>, String> {
    use crate::models::middleware::types::auth::AuthSidecarMiddleware;
    use crate::models::middleware::types::connect::AuraboxConnectMiddleware;
    use crate::models::middleware::types::jwtauth::JwtAuthMiddleware;
    use crate::models::middleware::types::metadata_transform::MetadataTransformMiddleware;
    use crate::models::middleware::types::path_filter::PathFilterMiddleware;
    use crate::models::middleware::types::transform::JoltTransformMiddleware;

    match middleware_type.to_lowercase().as_str() {
        "jwtauth" | "jwt_auth" => {
            let config = crate::models::middleware::types::jwtauth::parse_config(options)?;
            Ok(Box::new(JwtAuthMiddleware::new(config)))
        }
        "basic_auth" => {
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
        "path_filter" => {
            let config = crate::models::middleware::types::path_filter::parse_config(options)?;
            Ok(Box::new(PathFilterMiddleware::new(config)?))
        }
        "metadata_transform" => {
            let config =
                crate::models::middleware::types::metadata_transform::parse_config(options)?;
            Ok(Box::new(MetadataTransformMiddleware::new(config)?))
        }
        _ => Err(format!(
            "Unsupported built-in middleware type: {}",
            middleware_type
        )),
    }
}

/// Build middleware instances for a pipeline from configuration
/// Returns a vector of constructed middleware objects in the order of pipeline names
pub fn build_middleware_instances_for_pipeline(
    names: &[String],
    config: &Config,
) -> Result<Vec<Box<dyn Middleware>>, String> {
    let mut instances = Vec::new();

    for name in names {
        if let Some(middleware_instance) = config.middleware.get(name) {
            let middleware = middleware_instance.resolve_middleware().map_err(|err| {
                format!("Failed to resolve middleware instance '{}': {}", name, err)
            })?;
            instances.push(middleware);
        } else {
            // Fallback: if the name itself corresponds to a built-in middleware type,
            // allow referencing it directly without an instance block.
            // This supports conveniences like using "json_extractor" without an options table.
            let empty_opts: HashMap<String, Value> = HashMap::new();
            match resolve_middleware_type(name, &empty_opts) {
                Ok(mw) => instances.push(mw),
                Err(_) => {
                    return Err(format!("Unknown middleware instance '{}'", name));
                }
            }
        }
    }

    Ok(instances)
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

    /// Modify the response envelope coming from the backend.
    async fn right(
        &self,
        envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error>;
}
