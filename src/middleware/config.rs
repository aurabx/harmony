use serde::Deserialize;
use crate::middleware::jwtauth::JwtAuthConfig;
use crate::middleware::auth::AuthSidecarConfig;
use crate::middleware::connect::AuraboxConnectConfig;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct MiddlewareConfig {
    #[serde(default)]
    pub jwt_auth: Option<JwtAuthConfig>,
    #[serde(default)]
    pub auth_sidecar: Option<AuthSidecarConfig>,
    #[serde(default)]
    pub aurabox_connect: Option<AuraboxConnectConfig>,
}