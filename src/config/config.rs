use serde::Deserialize;
use crate::config::ConfigError;

#[derive(Debug, Deserialize, Default)]
pub struct ProxyConfig {
    pub id: String,
    pub log_level: String,
    pub store_dir: String,
}

impl ProxyConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.id.trim().is_empty() {
            return Err(ConfigError::InvalidProxyID);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct LoggingConfig {
    pub log_to_file: bool,
    pub log_file_path: String,
}