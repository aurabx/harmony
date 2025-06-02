
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

#[derive(Debug, Deserialize, Clone)]
pub struct JwtAuthConfig {
    pub jwks_url: String,
    pub audience: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthSidecarConfig {
    pub token_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuraboxConnectConfig {
    pub enabled: bool,
    pub fallback_timeout_ms: u64,
}