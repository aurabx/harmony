use crate::config::config::{Config, ConfigError};
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::instance::MiddlewareInstance;
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
        "dimse_to_jmix",
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

#[derive(Debug, serde::Deserialize, Default, Clone, PartialEq)]
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

/// Builds the options a middleware factory receives for instance `name`.
///
/// Merges the referenced authentication definition into the instance options
/// and injects `authentication_def` and `__instance_name`. Request handling
/// and config validation both call this, so a factory sees the same options
/// in both places.
pub fn resolve_instance_options(
    name: &str,
    instance: &MiddlewareInstance,
    config: &Config,
) -> Result<HashMap<String, Value>, String> {
    let mut resolved_options = instance.options.clone();
    if let Some(auth_ref) = &instance.authentication {
        let auth_def = config.authentications.get(auth_ref).ok_or_else(|| {
            format!(
                "Middleware '{}' references unknown authentication '{}'",
                name, auth_ref
            )
        })?;
        // Merge authentication options into middleware options (legacy behavior)
        for (key, value) in &auth_def.options {
            resolved_options.insert(key.clone(), value.clone());
        }
        // Inject full authentication definition so helpers can apply headers
        resolved_options.insert(
            "authentication_def".to_string(),
            serde_json::to_value(auth_def)
                .map_err(|e| format!("Failed to serialize auth def: {}", e))?,
        );
    }

    // Pass instance name for metadata lookup (webhook.<instance_name>)
    resolved_options.insert("__instance_name".to_string(), serde_json::json!(name));
    Ok(resolved_options)
}

/// Checks that middleware instance `name` can be built from `config`.
pub fn validate_middleware_instance(
    name: &str,
    instance: &MiddlewareInstance,
    config: &Config,
) -> Result<(), String> {
    let options = resolve_instance_options(name, instance, config)?;
    validate_middleware_options(name, &instance.middleware_type, &options, config)
}

/// Checks a pipeline middleware name that has no `[middleware.<name>]` block.
///
/// `build_middleware_instances_for_pipeline` treats such a name as a type
/// and builds it with empty options, so this validates exactly that.
pub fn validate_middleware_type_reference(name: &str, config: &Config) -> Result<(), String> {
    validate_middleware_options(name, name, &HashMap::new(), config)
}

/// Runs the registered validator for `middleware_type` against `options`.
///
/// Factories use `MiddlewareFactory::validate_config`, which runs the same
/// option resolution as request handling without side effects. Auth
/// providers use `AuthProvider::validate_config`. Returns `Ok` when nothing
/// is registered for the type: custom factories and providers are registered
/// by `HarmonyBuilder::run` after the first config load, and unknown built-in
/// types are rejected separately by `Config::validate_middleware_types`.
fn validate_middleware_options(
    name: &str,
    middleware_type: &str,
    options: &HashMap<String, Value>,
    config: &Config,
) -> Result<(), String> {
    crate::extensions::registry::initialize_builtin_factories_sync();
    let registry = crate::extensions::get_registry();
    let reg = registry
        .read()
        .map_err(|e| format!("Failed to acquire read lock on registry: {}", e))?;

    if let Some(factory) = reg.find_middleware_factory(middleware_type) {
        factory.validate_config(options, config)
    } else if let Some(provider) = reg.find_auth_provider(middleware_type) {
        provider.validate_config(options)
    } else {
        tracing::debug!(
            "Skipping option validation for middleware '{}': nothing registered for type '{}'",
            name,
            middleware_type
        );
        Ok(())
    }
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
            let resolved_options = resolve_instance_options(name, middleware_instance, config)?;

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
