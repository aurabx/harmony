use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbeamConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cloud_api_base_url: Option<String>,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval_secs() -> u64 {
    30
}

impl Default for RunbeamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cloud_api_base_url: None,
            poll_interval_secs: 30,
        }
    }
}

impl RunbeamConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.poll_interval_secs < 5 || self.poll_interval_secs > 3600 {
            return Err(format!(
                "runbeam.poll_interval_secs must be between 5 and 3600, got {}",
                self.poll_interval_secs
            ));
        }
        Ok(())
    }

    pub fn effective_cloud_api_base_url(&self) -> String {
        self.cloud_api_base_url
            .clone()
            .unwrap_or_else(|| "https://api.runbeam.cloud".to_string())
    }

    /// Get poll interval as Duration
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs)
    }
}
