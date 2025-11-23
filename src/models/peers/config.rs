use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct PeerConfig {
    pub id: Option<String>,
    pub name: Option<String>,
    pub connection: ConnectionConfig,
    pub r#type: String,
    pub description: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub tags: Option<Vec<String>>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: Option<u16>,
}

impl PeerConfig {
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

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_peer_config() {
        let config = PeerConfig {
            id: Some("peer1".to_string()),
            name: Some("Test Peer".to_string()),
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: Some(8080),
            },
            r#type: "http".to_string(),
            description: Some("A test peer".to_string()),
            enabled: true,
            tags: Some(vec!["tag1".to_string()]),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_peer_host() {
        let config = PeerConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "".to_string(),
                port: None,
            },
            r#type: "http".to_string(),
            description: None,
            enabled: true,
            tags: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_peer_type() {
        let config = PeerConfig {
            id: None,
            name: None,
            connection: ConnectionConfig {
                host: "example.com".to_string(),
                port: None,
            },
            r#type: "invalid_type".to_string(),
            description: None,
            enabled: true,
            tags: None,
        };
        assert!(config.validate().is_err());
    }
}
