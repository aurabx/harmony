use crate::models::middleware::types::auth::AuthSidecarConfig;
use crate::models::middleware::types::connect::AuraboxConnectConfig;
use crate::models::middleware::types::jwtauth::JwtAuthConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct MiddlewareConfig {
    #[serde(default)]
    pub jwt_auth: Option<JwtAuthConfig>,
    #[serde(default)]
    pub auth_sidecar: Option<AuthSidecarConfig>,
    #[serde(default)]
    pub aurabox_connect: Option<AuraboxConnectConfig>,
}
