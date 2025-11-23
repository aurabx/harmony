use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct TargetConfig {
    pub id: Option<String>,
    pub name: Option<String>,
    pub connection: ConnectionConfig,
    pub r#type: String,
    pub description: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub authentication: Option<AuthenticationConfig>,
    pub tags: Option<Vec<String>>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_enabled() -> bool {
    true
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_max_retries() -> u32 {
    3
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct AuthenticationConfig {
    #[serde(default = "default_auth_method")]
    pub method: String,
    pub credentials_path: Option<String>,
}

fn default_auth_method() -> String {
    "none".to_string()
}

impl TargetConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.connection.host.trim().is_empty() {
            return Err("connection.host cannot be empty".to_string());
        }

        let valid_types = ["http", "https", "dicom", "harmony", "fhir", "hl7v2", "custom"];
        if !valid_types.contains(&self.r#type.as_str()) {
            return Err(format!(
                "Invalid type '{}'. Must be one of: {:?}",
                self.r#type, valid_types
            ));
        }

        if let Some(auth) = &self.authentication {
            let valid_methods = ["none", "basic", "bearer", "api_key", "mutual_tls", "custom"];
            if !valid_methods.contains(&auth.method.as_str()) {
                return Err(format!(
                    "Invalid authentication method '{}'. Must be one of: {:?}",
                    auth.method, valid_methods
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_target_config() {
        let config = TargetConfig {
            id: Some("target1".to_string()),
            name: Some("Test Target".to_string()),
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: Some(8080),
            },
            r#type: "http".to_string(),
            description: Some("A test target".to_string()),
            enabled: true,
            authentication: Some(AuthenticationConfig {
                method: "basic".to_string(),
                credentials_path: Some("./creds".to_string()),
            }),
            tags: Some(vec!["tag1".to_string()]),
            timeout_secs: 30,
            max_retries: 3,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_target_host() {
        let config = TargetConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "".to_string(),
                port: None,
            },
            r#type: "http".to_string(),
            description: None,
            enabled: true,
            authentication: None,
            tags: None,
            timeout_secs: 30,
            max_retries: 3,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_target_type() {
        let config = TargetConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: None,
            },
            r#type: "invalid".to_string(),
            description: None,
            enabled: true,
            authentication: None,
            tags: None,
            timeout_secs: 30,
            max_retries: 3,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_auth_method() {
        let config = TargetConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: None,
            },
            r#type: "http".to_string(),
            description: None,
            enabled: true,
            authentication: Some(AuthenticationConfig {
                method: "invalid".to_string(),
                credentials_path: None,
            }),
            tags: None,
            timeout_secs: 30,
            max_retries: 3,
        };
        assert!(config.validate().is_err());
    }
}
