use serde::Deserialize;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ManagementConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_admin_base_path")]
    pub base_path: String,
    pub network: Option<String>,
    /// Interval in seconds for polling cloud API (default: 30s)
    /// Note: Polling starts automatically after gateway authorization via /admin/authorize
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Base URL for Runbeam Cloud API (optional override, defaults to https://api.runbeam.cloud)
    pub cloud_api_base_url: Option<String>,
}

pub fn default_admin_base_path() -> String {
    "admin".to_string()
}

fn default_poll_interval_secs() -> u64 {
    30
}

impl Default for ManagementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_path: default_admin_base_path(),
            network: None,
            poll_interval_secs: default_poll_interval_secs(),
            cloud_api_base_url: None,
        }
    }
}

impl ManagementConfig {
    /// Get poll interval as Duration
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs)
    }

    pub fn validate(&self) -> Result<(), String> {
        // Basic validation only - path must not be empty
        if self.base_path.trim().is_empty() {
            return Err("base_path cannot be empty".to_string());
        }
        
        // Validate poll interval
        if self.poll_interval_secs == 0 {
            return Err("poll_interval_secs must be greater than 0".to_string());
        }
        
        Ok(())
    }
}
