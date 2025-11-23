use serde::{Deserialize, Serialize};

/// Standardized connection configuration shared across components
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: Option<u16>,
    pub protocol: Option<String>,
    #[serde(default)]
    pub base_path: Option<String>,
}

/// Authentication configuration shared across components
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct AuthenticationConfig {
    #[serde(default = "default_auth_method")]
    pub method: String,
    pub credentials_path: Option<String>,
}

fn default_auth_method() -> String {
    "none".to_string()
}

/// Reliability configuration (timeout, retries)
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ReliabilityConfig {
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_max_retries() -> u32 {
    3
}

impl Default for ReliabilityConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout_secs(),
            max_retries: default_max_retries(),
        }
    }
}
