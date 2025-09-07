use serde::Deserialize;
use crate::models::middleware::jwtauth::JwtAuthConfig;
use crate::models::middleware::auth::AuthSidecarConfig;
use crate::models::middleware::connect::AuraboxConnectConfig;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct MiddlewareConfig {
    #[serde(default)]
    pub jwt_auth: Option<JwtAuthConfig>,
    #[serde(default)]
    pub auth_sidecar: Option<AuthSidecarConfig>,
    #[serde(default)]
    pub aurabox_connect: Option<AuraboxConnectConfig>,
}