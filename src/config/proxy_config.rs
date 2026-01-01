use serde::Deserialize;

/// Content parsing size limits
#[derive(Debug, Deserialize, Clone)]
pub struct ContentLimits {
    /// Maximum body size in bytes (default: 10MB)
    #[serde(default = "default_max_body_size")]
    pub max_body_size: usize,
    /// Maximum CSV rows to parse (default: 10,000)
    #[serde(default = "default_max_csv_rows")]
    pub max_csv_rows: usize,
    /// Maximum XML depth to prevent XML bombs (default: 100)
    #[serde(default = "default_max_xml_depth")]
    pub max_xml_depth: usize,
    /// Maximum multipart files per request (default: 10)
    #[serde(default = "default_max_multipart_files")]
    pub max_multipart_files: usize,
    /// Maximum form fields per request (default: 1,000)
    #[serde(default = "default_max_form_fields")]
    pub max_form_fields: usize,
}

impl Default for ContentLimits {
    fn default() -> Self {
        Self {
            max_body_size: default_max_body_size(),
            max_csv_rows: default_max_csv_rows(),
            max_xml_depth: default_max_xml_depth(),
            max_multipart_files: default_max_multipart_files(),
            max_form_fields: default_max_form_fields(),
        }
    }
}

/// Represents the configuration for the proxy
#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    pub id: String,
    #[serde(default = "default_pipelines_path")]
    pub pipelines_path: String,
    #[serde(default = "default_transforms_path")]
    pub transforms_path: String,
    #[serde(default = "default_mesh_path")]
    pub mesh_path: String,
    /// Duration (in hours) to cache JWKS keys fetched from Runbeam Cloud
    #[serde(default = "default_jwks_cache_duration_hours")]
    pub jwks_cache_duration_hours: u64,
    /// Content parsing size limits
    #[serde(default)]
    pub content_limits: ContentLimits,
    /// List of required environment variables that must be present for configuration to be valid
    #[serde(default)]
    pub required_env_vars: Vec<String>,
    /// Field name patterns to treat as sensitive in logs (e.g., "*_key", "secret")
    #[serde(default)]
    pub sensitive_field_patterns: Vec<String>,
    /// Primary provider for cloud polling settings (defaults to "runbeam")
    #[serde(default = "default_primary_provider")]
    pub primary_provider: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            pipelines_path: default_pipelines_path(),
            transforms_path: default_transforms_path(),
            mesh_path: default_mesh_path(),
            jwks_cache_duration_hours: default_jwks_cache_duration_hours(),
            content_limits: ContentLimits::default(),
            required_env_vars: Vec::new(),
            sensitive_field_patterns: Vec::new(),
            primary_provider: default_primary_provider(),
        }
    }
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

/// Default mesh path for the proxy configuration
fn default_mesh_path() -> String {
    // Resolved relative to the directory of the base config file
    "mesh".to_string()
}

/// Default JWKS cache duration (24 hours)
fn default_jwks_cache_duration_hours() -> u64 {
    24
}

/// Default maximum body size (10 MB)
fn default_max_body_size() -> usize {
    10 * 1024 * 1024
}

/// Default maximum CSV rows
fn default_max_csv_rows() -> usize {
    10_000
}

/// Default maximum XML depth
fn default_max_xml_depth() -> usize {
    100
}

/// Default maximum multipart files
fn default_max_multipart_files() -> usize {
    10
}

/// Default maximum form fields
fn default_max_form_fields() -> usize {
    1_000
}

/// Default primary provider
fn default_primary_provider() -> String {
    "runbeam".to_string()
}

impl ProxyConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("proxy.id cannot be empty".to_string());
        }

        if self.jwks_cache_duration_hours < 1 || self.jwks_cache_duration_hours > 168 {
            return Err(format!(
                "proxy.jwks_cache_duration_hours must be between 1 and 168, got {}",
                self.jwks_cache_duration_hours
            ));
        }

        // Validate content limits
        if self.content_limits.max_body_size == 0 {
            return Err("proxy.content_limits.max_body_size must be greater than 0".to_string());
        }

        if self.content_limits.max_csv_rows == 0 {
            return Err("proxy.content_limits.max_csv_rows must be greater than 0".to_string());
        }

        if self.content_limits.max_xml_depth == 0 {
            return Err("proxy.content_limits.max_xml_depth must be greater than 0".to_string());
        }

        if self.content_limits.max_multipart_files == 0 {
            return Err(
                "proxy.content_limits.max_multipart_files must be greater than 0".to_string(),
            );
        }

        if self.content_limits.max_form_fields == 0 {
            return Err("proxy.content_limits.max_form_fields must be greater than 0".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primary_provider_default() {
        let config: ProxyConfig = toml::from_str(r#"
            id = "test-proxy"
        "#).unwrap();
        assert_eq!(config.primary_provider, "runbeam");
    }

    #[test]
    fn test_primary_provider_custom() {
        let config: ProxyConfig = toml::from_str(r#"
            id = "test-proxy"
            primary_provider = "custom_provider"
        "#).unwrap();
        assert_eq!(config.primary_provider, "custom_provider");
    }

    #[test]
    fn test_primary_provider_local() {
        let config: ProxyConfig = toml::from_str(r#"
            id = "test-proxy"
            primary_provider = "local"
        "#).unwrap();
        assert_eq!(config.primary_provider, "local");
    }
}
