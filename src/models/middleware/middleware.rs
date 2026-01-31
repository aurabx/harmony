use crate::config::config::{Config, ConfigError};
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::utils::Error;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::Value;
use std::collections::HashMap;

// Middleware registry similar to services
pub static MIDDLEWARE_REGISTRY: OnceCell<HashMap<String, String>> = OnceCell::new();

/// Returns all valid built-in middleware type names.
/// This is the single source of truth for middleware type validation.
pub fn builtin_middleware_types() -> &'static [&'static str] {
    &[
        "jwtauth",
        "jwt_auth",
        "basic_auth",
        "connect",
        "passthru",
        "json_extractor",
        "json",
        "jmix_builder",
        "dicomweb_bridge",
        "dicomweb",
        "dicom_to_dicomweb",
        "dicom_flatten",
        "dicom_flatten_middleware",
        "dicom_unflatten",
        "dicom_unflatten_middleware",
        "transform",
        "path_filter",
        "log_dump",
        "dump",
        "webhook",
        "metadata_transform",
        "policies",
        "mesh_auth",
    ]
}

#[derive(Debug, serde::Deserialize, Default, Clone)]
#[serde(default)]
pub struct MiddlewareConfig {
    pub module: String, // Path to the module or metadata
}

pub fn initialise_middleware_registry(config: &Config) {
    // Populate the registry using middleware types from the provided config
    // Use get_or_init to allow safe re-initialization in tests
    MIDDLEWARE_REGISTRY.get_or_init(|| {
        config
            .middleware_types
            .iter()
            .map(|(key, value)| (key.clone(), value.module.clone()))
            .collect()
    });
}

/// Resolves a middleware type from the registry and returns a boxed Middleware
pub fn resolve_middleware_type(
    middleware_type: &str,
    options: &HashMap<String, Value>,
    transforms_path: Option<&str>,
) -> Result<Box<dyn Middleware>, String> {
    resolve_middleware_type_with_config(middleware_type, options, transforms_path, None)
}

/// Resolves a middleware type with full Config context (for policies middleware)
pub fn resolve_middleware_type_with_config(
    middleware_type: &str,
    options: &HashMap<String, Value>,
    _transforms_path: Option<&str>,
    config: Option<&crate::config::config::Config>,
) -> Result<Box<dyn Middleware>, String> {
    // Ensure built-in factories are registered (idempotent, thread-safe)
    crate::extensions::registry::initialize_builtin_factories_sync();

    // Use the unified registry for all middleware resolution
    let registry = crate::extensions::get_registry();
    let reg = registry
        .read()
        .map_err(|e| format!("Failed to acquire read lock on registry: {}", e))?;

    reg.resolve_middleware(middleware_type, options, config)
}

/// Build middleware instances for a pipeline from configuration
/// Returns a vector of constructed middleware objects in the order of pipeline names
pub fn build_middleware_instances_for_pipeline(
    names: &[String],
    config: &Config,
) -> Result<Vec<Box<dyn Middleware>>, String> {
    let mut instances = Vec::new();
    let transforms_path = config.resolved_transforms_path.as_deref();

    for name in names {
        if let Some(middleware_instance) = config.middleware.get(name) {
            // Resolve authentication reference if present
            let mut resolved_options = middleware_instance.options.clone();
            if let Some(auth_ref) = &middleware_instance.authentication {
                if let Some(auth_def) = config.authentications.get(auth_ref) {
                    // Merge authentication options into middleware options (legacy behavior)
                    for (key, value) in &auth_def.options {
                        resolved_options.insert(key.clone(), value.clone());
                    }
                    // Inject full authentication definition so helpers can apply headers
                    resolved_options.insert(
                        "authentication_def".to_string(),
                        serde_json::to_value(auth_def).map_err(|e| format!("Failed to serialize auth def: {}", e))?,
                    );
                } else {
                    return Err(format!(
                        "Middleware '{}' references unknown authentication '{}'",
                        name, auth_ref
                    ));
                }
            }

            // Pass instance name for metadata lookup (webhook.<instance_name>)
            resolved_options.insert("__instance_name".to_string(), serde_json::json!(name));

            // Use new method that passes Config context for policies middleware
            let middleware = resolve_middleware_type_with_config(
                &middleware_instance.middleware_type,
                &resolved_options,
                transforms_path,
                Some(config),
            )
            .map_err(|err| format!("Failed to resolve middleware instance '{}': {}", name, err))?;
            instances.push(middleware);
        } else {
            // Fallback: if the name itself corresponds to a built-in middleware type,
            // allow referencing it directly without an instance block.
            // This supports conveniences like using "json_extractor" without an options table.
            let empty_opts: HashMap<String, Value> = HashMap::new();
            match resolve_middleware_type_with_config(
                name,
                &empty_opts,
                transforms_path,
                Some(config),
            ) {
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
