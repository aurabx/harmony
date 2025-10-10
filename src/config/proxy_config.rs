use serde::Deserialize;

/// Represents the configuration for the proxy
#[derive(Debug, Deserialize, Default)]
pub struct ProxyConfig {
    pub id: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_pipelines_path")]
    pub pipelines_path: String,
    #[serde(default = "default_transforms_path")]
    pub transforms_path: String,
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
