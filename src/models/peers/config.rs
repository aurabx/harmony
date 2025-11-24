use crate::models::connection::ConnectionConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct PeerConfig {
    pub id: Option<String>,
    pub name: Option<String>,
    pub connection: ConnectionConfig,
    #[serde(alias = "type")]
    pub protocol: Option<String>,
    pub description: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Authentication reference (DSL v1.9.0+): ID of global authentication definition
    pub authentication: Option<String>,
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

impl PeerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.connection.host.trim().is_empty() {
            return Err("connection.host cannot be empty".to_string());
        }

        let protocol = self.get_protocol();
        if protocol.is_none() {
             return Err("Protocol must be specified either in 'protocol' (or 'type') field or 'connection.protocol'".to_string());
        }
        let protocol = protocol.unwrap();

        let valid_types = ["http", "https", "dicom", "harmony", "fhir", "hl7v2", "custom"];
        if !valid_types.contains(&protocol.as_str()) {
            return Err(format!(
                "Invalid protocol '{}'. Must be one of: {:?}",
                protocol, valid_types
            ));
        }

        // Authentication reference validation is deferred to config resolution phase
        // where the global authentications map is available

        Ok(())
    }

    pub fn get_protocol(&self) -> Option<String> {
        self.protocol
            .clone()
            .or_else(|| self.connection.protocol.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::connection::ConnectionConfig;

    #[test]
    fn test_valid_peer_config() {
        let config = PeerConfig {
            id: Some("peer1".to_string()),
            name: Some("Test Peer".to_string()),
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: Some(8080),
                protocol: None,
                base_path: None,
            },
            protocol: Some("http".to_string()),
            description: Some("A test peer".to_string()),
            enabled: true,
            authentication: Some("authentications.basic-auth".to_string()),
            tags: Some(vec!["tag1".to_string()]),
            timeout_secs: 30,
            max_retries: 3,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_valid_peer_config_with_connection_protocol() {
        let config = PeerConfig {
            id: Some("peer1".to_string()),
            name: Some("Test Peer".to_string()),
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: Some(8080),
                protocol: Some("http".to_string()),
                base_path: None,
            },
            protocol: None,
            description: Some("A test peer".to_string()),
            enabled: true,
            authentication: None,
            tags: None,
            timeout_secs: 30,
            max_retries: 3,
        };
        assert!(config.validate().is_ok());
        assert_eq!(config.get_protocol(), Some("http".to_string()));
    }

    #[test]
    fn test_invalid_peer_host() {
        let config = PeerConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "".to_string(),
                port: None,
                protocol: None,
                base_path: None,
            },
            protocol: Some("http".to_string()),
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
    fn test_invalid_peer_protocol() {
        let config = PeerConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: None,
                protocol: None,
                base_path: None,
            },
            protocol: Some("invalid_type".to_string()),
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
    fn test_missing_protocol() {
        let config = PeerConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: None,
                protocol: None,
                base_path: None,
            },
            protocol: None,
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
    fn test_with_auth_reference() {
        // Auth reference validation happens during config resolution, not during struct validation
        let config = PeerConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: None,
                protocol: None,
                base_path: None,
            },
            protocol: Some("http".to_string()),
            description: None,
            enabled: true,
            authentication: Some("authentications.some-auth".to_string()),
            tags: None,
            timeout_secs: 30,
            max_retries: 3,
        };
        // Should pass validation - auth reference is resolved later
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_type_alias_deserialization() {
        let json = r#"{
            "connection": { "host": "example.com" },
            "type": "http"
        }"#;
        let config: PeerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.protocol, Some("http".to_string()));
    }
}
