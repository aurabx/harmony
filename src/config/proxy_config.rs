use serde::Deserialize;

/// Represents the configuration for the proxy
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ProxyConfig {
    pub id: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_pipelines_path")]
    pub pipelines_path: String,
    #[serde(default = "default_transforms_path")]
    pub transforms_path: String,
    /// Duration (in hours) to cache JWKS keys fetched from Runbeam Cloud
    #[serde(default = "default_jwks_cache_duration_hours")]
    pub jwks_cache_duration_hours: u64,
}

/// Default log level for the proxy configuration
fn default_log_level() -> String {
    "error".to_string()
}

/// Default pipelines path for the proxy configuration
fn default_pipelines_path() -> String {
    // Resolved relative to the directory of the base config file
    "pipelines".to_string()
}

/// Default transforms path for the proxy configuration
fn default_transforms_path() -> String {
    // Resolved relative to the directory of the base config file
    "transforms".to_string()
}

/// Default JWKS cache duration (24 hours)
fn default_jwks_cache_duration_hours() -> u64 {
    24
}
