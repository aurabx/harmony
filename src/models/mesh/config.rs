use serde::{Deserialize, Serialize};
use std::fmt;

/// Protocol type for mesh communication
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshProtocol {
    Http,
    Http3,
}

impl Default for MeshProtocol {
    fn default() -> Self {
        MeshProtocol::Http
    }
}

impl fmt::Display for MeshProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshProtocol::Http => write!(f, "http"),
            MeshProtocol::Http3 => write!(f, "http3"),
        }
    }
}

/// Mesh provider type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshProvider {
    /// Self-managed mesh with local certificates
    Local,
    /// Runbeam Cloud managed mesh
    Runbeam,
}

impl Default for MeshProvider {
    fn default() -> Self {
        MeshProvider::Local
    }
}

impl fmt::Display for MeshProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshProvider::Local => write!(f, "local"),
            MeshProvider::Runbeam => write!(f, "runbeam"),
        }
    }
}

/// Mesh definition linking ingress and egress points for inter-proxy communication
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    /// Optional unique identifier for this mesh
    #[serde(default)]
    pub id: Option<String>,

    /// Protocol type for mesh communication
    #[serde(rename = "type")]
    pub mesh_type: MeshProtocol,

    /// Mesh provider - local or runbeam
    pub provider: MeshProvider,

    /// List of ingress definition names that belong to this mesh
    #[serde(default)]
    pub ingress: Vec<String>,

    /// List of egress definition names that belong to this mesh
    #[serde(default)]
    pub egress: Vec<String>,

    /// Human-readable description of this mesh
    #[serde(default)]
    pub description: Option<String>,

    /// Whether this mesh is currently active
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl Default for Mesh {
    fn default() -> Self {
        Self {
            id: None,
            mesh_type: MeshProtocol::default(),
            provider: MeshProvider::default(),
            ingress: Vec::new(),
            egress: Vec::new(),
            description: None,
            enabled: true,
        }
    }
}

impl Mesh {
    /// Validate the mesh configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.ingress.is_empty() {
            return Err("Mesh must have at least one ingress".to_string());
        }
        if self.egress.is_empty() {
            return Err("Mesh must have at least one egress".to_string());
        }
        Ok(())
    }
}

/// Ingress definition - allows other mesh members to send requests to this proxy
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshIngress {
    /// Optional unique identifier for this ingress
    #[serde(default)]
    pub id: Option<String>,

    /// Protocol type for incoming mesh requests
    #[serde(rename = "type")]
    pub ingress_type: MeshProtocol,

    /// Reference to an endpoint name that incoming mesh requests will be routed to
    pub endpoint: String,

    /// List of URLs that map to this ingress
    #[serde(default)]
    pub urls: Vec<String>,

    /// Human-readable description of this ingress point
    #[serde(default)]
    pub description: Option<String>,

    /// Whether this ingress is currently active
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl Default for MeshIngress {
    fn default() -> Self {
        Self {
            id: None,
            ingress_type: MeshProtocol::default(),
            endpoint: String::new(),
            urls: Vec::new(),
            description: None,
            enabled: true,
        }
    }
}

impl MeshIngress {
    /// Validate the ingress configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.endpoint.trim().is_empty() {
            return Err("Ingress must reference an endpoint".to_string());
        }
        if self.urls.is_empty() {
            return Err("Ingress must have at least one URL".to_string());
        }
        Ok(())
    }
}

/// Egress definition - allows this proxy to send requests to other mesh members
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshEgress {
    /// Optional unique identifier for this egress
    #[serde(default)]
    pub id: Option<String>,

    /// Protocol type for outgoing mesh requests
    #[serde(rename = "type")]
    pub egress_type: MeshProtocol,

    /// Reference to a backend name that outgoing mesh requests will be routed through
    pub backend: String,

    /// Human-readable description of this egress point
    #[serde(default)]
    pub description: Option<String>,

    /// Whether this egress is currently active
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl Default for MeshEgress {
    fn default() -> Self {
        Self {
            id: None,
            egress_type: MeshProtocol::default(),
            backend: String::new(),
            description: None,
            enabled: true,
        }
    }
}

impl MeshEgress {
    /// Validate the egress configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.backend.trim().is_empty() {
            return Err("Egress must reference a backend".to_string());
        }
        Ok(())
    }
}

fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mesh_protocol_serialization() {
        let http = MeshProtocol::Http;
        let http3 = MeshProtocol::Http3;

        assert_eq!(serde_json::to_string(&http).unwrap(), "\"http\"");
        assert_eq!(serde_json::to_string(&http3).unwrap(), "\"http3\"");
    }

    #[test]
    fn test_mesh_provider_serialization() {
        let local = MeshProvider::Local;
        let runbeam = MeshProvider::Runbeam;

        assert_eq!(serde_json::to_string(&local).unwrap(), "\"local\"");
        assert_eq!(serde_json::to_string(&runbeam).unwrap(), "\"runbeam\"");
    }

    #[test]
    fn test_mesh_validation() {
        let mut mesh = Mesh::default();
        assert!(mesh.validate().is_err());

        mesh.ingress.push("ingress1".to_string());
        assert!(mesh.validate().is_err());

        mesh.egress.push("egress1".to_string());
        assert!(mesh.validate().is_ok());
    }

    #[test]
    fn test_ingress_validation() {
        let mut ingress = MeshIngress::default();
        assert!(ingress.validate().is_err());

        ingress.endpoint = "my-endpoint".to_string();
        assert!(ingress.validate().is_err());

        ingress.urls.push("https://example.com".to_string());
        assert!(ingress.validate().is_ok());
    }

    #[test]
    fn test_egress_validation() {
        let mut egress = MeshEgress::default();
        assert!(egress.validate().is_err());

        egress.backend = "my-backend".to_string();
        assert!(egress.validate().is_ok());
    }

    #[test]
    fn test_mesh_deserialization() {
        let toml_str = r#"
            type = "http3"
            provider = "runbeam"
            ingress = ["ingress1", "ingress2"]
            egress = ["egress1"]
            description = "Test mesh"
            enabled = true
        "#;

        let mesh: Mesh = toml::from_str(toml_str).unwrap();
        assert_eq!(mesh.mesh_type, MeshProtocol::Http3);
        assert_eq!(mesh.provider, MeshProvider::Runbeam);
        assert_eq!(mesh.ingress.len(), 2);
        assert_eq!(mesh.egress.len(), 1);
        assert_eq!(mesh.description, Some("Test mesh".to_string()));
        assert!(mesh.enabled);
    }

    #[test]
    fn test_ingress_deserialization() {
        let toml_str = r#"
            type = "http"
            endpoint = "api-endpoint"
            urls = ["https://api.example.com", "https://api2.example.com"]
        "#;

        let ingress: MeshIngress = toml::from_str(toml_str).unwrap();
        assert_eq!(ingress.ingress_type, MeshProtocol::Http);
        assert_eq!(ingress.endpoint, "api-endpoint");
        assert_eq!(ingress.urls.len(), 2);
    }

    #[test]
    fn test_egress_deserialization() {
        let toml_str = r#"
            type = "http3"
            backend = "remote-backend"
            description = "Egress to remote service"
        "#;

        let egress: MeshEgress = toml::from_str(toml_str).unwrap();
        assert_eq!(egress.egress_type, MeshProtocol::Http3);
        assert_eq!(egress.backend, "remote-backend");
        assert_eq!(egress.description, Some("Egress to remote service".to_string()));
    }
}
