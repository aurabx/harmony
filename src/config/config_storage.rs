use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration stored in ~/.runbeam/<proxy_id>/config.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
}

/// Get the directory path for proxy instance config: ~/.runbeam/<proxy_id>
fn get_proxy_dir(proxy_id: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".runbeam").join(proxy_id))
}

/// Get the config file path: ~/.runbeam/<proxy_id>/config.json
fn get_config_path(proxy_id: &str) -> Option<PathBuf> {
    get_proxy_dir(proxy_id).map(|dir| dir.join("config.json"))
}

/// Load proxy config from ~/.runbeam/<proxy_id>/config.json
///
/// Returns None if file doesn't exist or can't be read/parsed
pub fn load_config(proxy_id: &str) -> Option<ProxyConfig> {
    let config_path = get_config_path(proxy_id)?;
    
    let content = std::fs::read_to_string(&config_path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Save proxy config to ~/.runbeam/<proxy_id>/config.json
///
/// Creates the directory structure if it doesn't exist
pub fn save_config(proxy_id: &str, config: &ProxyConfig) -> Result<(), String> {
    let proxy_dir = get_proxy_dir(proxy_id)
        .ok_or_else(|| "Failed to determine home directory".to_string())?;
    
    let config_path = proxy_dir.join("config.json");
    
    // Create directory if it doesn't exist
    std::fs::create_dir_all(&proxy_dir)
        .map_err(|e| format!("Failed to create proxy directory: {}", e))?;
    
    // Serialize and write config
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    
    std::fs::write(&config_path, json)
        .map_err(|e| format!("Failed to write config file: {}", e))?;
    
    tracing::info!("Saved proxy config to: {:?}", config_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_config_serialization() {
        let config = ProxyConfig {
            api_base_url: Some("https://api.example.com".to_string()),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("api_base_url"));
        assert!(json.contains("https://api.example.com"));

        let deserialized: ProxyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.api_base_url,
            Some("https://api.example.com".to_string())
        );
    }

    #[test]
    fn test_proxy_config_default() {
        let config = ProxyConfig::default();
        assert_eq!(config.api_base_url, None);
    }

    #[test]
    fn test_proxy_config_skips_none() {
        let config = ProxyConfig { api_base_url: None };

        let json = serde_json::to_string(&config).unwrap();
        // Should serialize to empty object when None
        assert_eq!(json, "{}");
    }
}
