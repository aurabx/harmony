use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct MiddlewareInstance {
    #[serde(rename = "type")]
    pub middleware_type: String,
    /// Authentication reference (DSL v1.9.0+): ID of global authentication definition
    /// Used for auth-related middleware (jwt_auth, basic_auth)
    pub authentication: Option<String>,
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

impl MiddlewareInstance {
    /// Resolves the middleware type using the centralized middleware resolver
    pub fn resolve_middleware(
        &self,
        transforms_path: Option<&str>,
    ) -> Result<Box<dyn crate::models::middleware::middleware::Middleware>, String> {
        crate::models::middleware::middleware::resolve_middleware_type(
            &self.middleware_type,
            &self.options,
            transforms_path,
        )
    }
}

