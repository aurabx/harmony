use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManagementConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_admin_base_path")]
    pub base_path: String,
}

pub fn default_admin_base_path() -> String {
    "admin".to_string()
}

impl Default for ManagementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_path: default_admin_base_path(),
        }
    }
}

impl ManagementConfig {
    pub fn validate(&self) -> Result<(), String> {
        // Basic validation only - path must not be empty
        if self.base_path.trim().is_empty() {
            return Err("base_path cannot be empty".to_string());
        }
        Ok(())
    }
}
