use std::collections::HashMap;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct MiddlewareInstance {
    #[serde(rename = "type")]
    pub middleware_type: String,
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

impl MiddlewareInstance {
    /// Resolves the middleware type using the centralized middleware resolver
    pub fn resolve_middleware(&self) -> Result<Box<dyn crate::models::middleware::middleware::Middleware>, String> {
        crate::models::middleware::middleware::resolve_middleware_type(&self.middleware_type, &self.options)
    }
}

// Rename the existing MiddlewareConfig to avoid confusion
#[derive(Debug, Deserialize, Default, Clone)]
pub struct MiddlewareInstanceConfig {
    #[serde(default)]
    pub jwt_auth: Option<crate::models::middleware::types::jwtauth::JwtAuthConfig>,
    #[serde(default)]
    pub auth_sidecar: Option<crate::models::middleware::types::auth::AuthSidecarConfig>,
    #[serde(default)]
    pub aurabox_connect: Option<crate::models::middleware::types::connect::AuraboxConnectConfig>,
}