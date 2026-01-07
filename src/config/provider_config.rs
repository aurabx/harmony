use serde::{Deserialize, Serialize};

/// Provider configuration for resource resolution.
///
/// Providers define how resources are resolved - either locally from config files
/// or remotely from a provider API (e.g., Runbeam Cloud).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Base URL for provider API. Required for remote providers, omitted for 'local'.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,

    /// Polling interval in seconds for change detection
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval_secs() -> u64 {
    30
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api: None,
            poll_interval_secs: 30,
        }
    }
}

impl ProviderConfig {
    /// Returns true if this is a remote provider (has an API URL)
    pub fn is_remote(&self) -> bool {
        self.api.is_some()
    }

    /// Validate the provider configuration
    pub fn validate(&self, name: &str) -> Result<(), String> {
        // Remote providers must have an API URL
        if name != "local" && self.api.is_none() {
            return Err(format!(
                "Provider '{}' requires an 'api' field (only 'local' can omit it)",
                name
            ));
        }

        Ok(())
    }

    /// Returns true if polling is enabled (poll_interval_secs > 0)
    pub fn polling_enabled(&self) -> bool {
        self.poll_interval_secs > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_config_defaults() {
        let config = ProviderConfig::default();
        assert_eq!(config.poll_interval_secs, 30);
        assert!(config.api.is_none());
        assert!(!config.is_remote());
    }

    #[test]
    fn test_provider_config_with_api() {
        let config = ProviderConfig {
            api: Some("https://api.runbeam.io".to_string()),
            poll_interval_secs: 60,
        };
        assert!(config.is_remote());
    }

    #[test]
    fn test_provider_config_validation() {
        // Local provider without API is valid
        let local = ProviderConfig::default();
        assert!(local.validate("local").is_ok());

        // Remote provider without API is invalid
        let remote_no_api = ProviderConfig::default();
        assert!(remote_no_api.validate("runbeam").is_err());

        // Remote provider with API is valid
        let remote_with_api = ProviderConfig {
            api: Some("https://api.runbeam.io".to_string()),
            ..Default::default()
        };
        assert!(remote_with_api.validate("runbeam").is_ok());
    }

    #[test]
    fn test_polling_enabled() {
        // Default has polling enabled
        let config = ProviderConfig::default();
        assert!(config.polling_enabled());

        // Zero disables polling
        let disabled = ProviderConfig {
            api: Some("https://api.runbeam.io".to_string()),
            poll_interval_secs: 0,
        };
        assert!(!disabled.polling_enabled());
    }

    #[test]
    fn test_provider_config_deserialization() {
        let toml_str = r#"
            api = "https://api.runbeam.io"
            poll_interval_secs = 60
        "#;

        let config: ProviderConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.api, Some("https://api.runbeam.io".to_string()));
        assert_eq!(config.poll_interval_secs, 60);
    }

    #[test]
    fn test_local_provider_deserialization() {
        // Local provider can omit api field
        let toml_str = "";

        let config: ProviderConfig = toml::from_str(toml_str).unwrap();
        assert!(config.api.is_none());
    }
}
