use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    pub id: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_store_dir")]
    pub store_dir: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            id: "".to_string(),
            log_level: default_log_level(),
            store_dir: default_store_dir(),
        }
    }
}

fn default_log_level() -> String {
    "error".to_string()
}

fn default_store_dir() -> String {
    "/tmp/harmony".to_string()
}

