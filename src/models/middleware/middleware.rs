
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use crate::models::envelope::envelope::Envelope;
use crate::utils::Error;
use crate::config::config::{Config, ConfigError};
use std::collections::HashMap;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::Value;

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
pub fn resolve_middleware(middleware_type: &str, options: &HashMap<String, Value>) -> Result<Box<dyn Middleware>, String> {
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
            Err(format!("Unknown middleware type: {}", middleware_type))
        }
    } else {
        // Fallback to hardcoded types if registry isn't initialized
        create_builtin_middleware(middleware_type, options)
    }
}

/// Creates built-in middleware instances
fn create_builtin_middleware(middleware_type: &str, options: &HashMap<String, Value>) -> Result<Box<dyn Middleware>, String> {
    use crate::models::middleware::types::auth::AuthSidecarMiddleware;
    use crate::models::middleware::types::connect::AuraboxConnectMiddleware;
    use crate::models::middleware::types::jwtauth::JwtAuthMiddleware;

    match middleware_type.to_lowercase().as_str() {
        "jwtauth" => {
            let config = parse_jwt_auth_config(options)?;
            Ok(Box::new(JwtAuthMiddleware::new(config)))
        },
        "auth" => {
            let config = parse_auth_sidecar_config(options)?;
            Ok(Box::new(AuthSidecarMiddleware::new(config)))
        },
        "connect" => {
            let config = parse_aurabox_connect_config(options)?;
            Ok(Box::new(AuraboxConnectMiddleware::new(config)))
        },
        "passthru" => {
            Ok(Box::new(crate::models::middleware::types::passthru::PassthruMiddleware::new()))
        }
        _ => Err(format!("Unsupported built-in middleware type: {}", middleware_type)),
    }
}

fn parse_jwt_auth_config(options: &HashMap<String, Value>) -> Result<crate::models::middleware::types::jwtauth::JwtAuthConfig, String> {
    let public_key_path = options
        .get("public_key_path")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'public_key_path' for jwtauth middleware")?;

    Ok(crate::models::middleware::types::jwtauth::JwtAuthConfig {
        public_key_path: public_key_path.to_string(),
    })
}

fn parse_auth_sidecar_config(options: &HashMap<String, Value>) -> Result<crate::models::middleware::types::auth::AuthSidecarConfig, String> {
    let token_path = options
        .get("token_path")
        .and_then(|v| v.as_str())
        .unwrap_or("").to_string();

    let username = options
        .get("username")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'username' for auth middleware")?;

    let password = options
        .get("password")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'password' for auth middleware")?;

    Ok(crate::models::middleware::types::auth::AuthSidecarConfig {
        token_path,
        username: username.to_string(),
        password: password.to_string(),
    })
}

fn parse_aurabox_connect_config(options: &HashMap<String, Value>) -> Result<crate::models::middleware::types::connect::AuraboxConnectConfig, String> {
    let enabled = options
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let fallback_timeout_ms = options
        .get("fallback_timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000);

    Ok(crate::models::middleware::types::connect::AuraboxConnectConfig {
        enabled,
        fallback_timeout_ms,
    })
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
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error>;

    /// Modify the incoming envelope on its way from the backend.
    async fn right(
        &self,
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error>;
}