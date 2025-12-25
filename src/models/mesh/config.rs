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

/// Authentication type for mesh communication
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshAuthType {
    /// JWT-based authentication
    Jwt,
}

impl Default for MeshAuthType {
    fn default() -> Self {
        MeshAuthType::Jwt
    }
}

impl fmt::Display for MeshAuthType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshAuthType::Jwt => write!(f, "jwt"),
        }
    }
}

/// Mode for ingress/egress operation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IngressEgressMode {
    /// Default mode: requests proceed regardless of mesh match
    Default,
    /// Mesh mode: only requests with valid mesh authentication are accepted
    Mesh,
}

impl Default for IngressEgressMode {
    fn default() -> Self {
        IngressEgressMode::Default
    }
}

impl fmt::Display for IngressEgressMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IngressEgressMode::Default => write!(f, "default"),
            IngressEgressMode::Mesh => write!(f, "mesh"),
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

    /// Authentication type for mesh members (default: jwt)
    #[serde(default)]
    pub auth_type: MeshAuthType,

    /// JWT secret for local provider (HS256)
    /// Used for both signing (egress) and verification (ingress)
    #[serde(default)]
    pub jwt_secret: Option<String>,

    /// Path to RSA private key for local provider (RS256)
    /// Used for signing JWTs on egress
    #[serde(default)]
    pub jwt_private_key_path: Option<String>,

    /// Path to RSA public key for local provider (RS256)
    /// Used for verifying JWTs on ingress
    #[serde(default)]
    pub jwt_public_key_path: Option<String>,

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
            auth_type: MeshAuthType::default(),
            jwt_secret: None,
            jwt_private_key_path: None,
            jwt_public_key_path: None,
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
        // Empty ingress/egress is valid - the mesh simply won't match anything
        Ok(())
    }
}

/// Ingress definition - allows other mesh members to send requests to this proxy
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshIngress {
    /// Optional unique identifier for this ingress
    #[serde(default)]
    pub id: Option<String>,

    /// Pipeline name that owns this ingress (required)
    pub pipeline: String,

    /// Protocol type for incoming mesh requests
    #[serde(rename = "type")]
    pub ingress_type: MeshProtocol,

    /// Mode for this ingress point
    /// - default: requests proceed regardless of mesh match
    /// - mesh: only requests with valid mesh authentication are accepted
    #[serde(default)]
    pub mode: IngressEgressMode,

    /// Optional endpoint override. If omitted, the first endpoint in the pipeline is used.
    #[serde(default)]
    pub endpoint: Option<String>,

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
            pipeline: String::new(),
            ingress_type: MeshProtocol::default(),
            mode: IngressEgressMode::default(),
            endpoint: None,
            urls: Vec::new(),
            description: None,
            enabled: true,
        }
    }
}

impl MeshIngress {
    /// Validate the ingress configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.pipeline.trim().is_empty() {
            return Err("Ingress must reference a pipeline".to_string());
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

    /// Pipeline name that owns this egress (required)
    pub pipeline: String,

    /// Protocol type for outgoing mesh requests
    #[serde(rename = "type")]
    pub egress_type: MeshProtocol,

    /// Mode for this egress point
    /// - default: requests proceed regardless of mesh match
    /// - mesh: only requests to mesh destinations are allowed
    #[serde(default)]
    pub mode: IngressEgressMode,

    /// Optional backend override. If omitted, the first backend in the pipeline is used.
    #[serde(default)]
    pub backend: Option<String>,

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
            pipeline: String::new(),
            egress_type: MeshProtocol::default(),
            mode: IngressEgressMode::default(),
            backend: None,
            description: None,
            enabled: true,
        }
    }
}

impl MeshEgress {
    /// Validate the egress configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.pipeline.trim().is_empty() {
            return Err("Egress must reference a pipeline".to_string());
        }
        Ok(())
    }
}

fn default_enabled() -> bool {
    true
}

/// Remote ingress definition - URLs of remote mesh members that this proxy can send to
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteIngress {
    /// List of URLs served by the remote ingress
    pub urls: Vec<String>,
}

impl RemoteIngress {
    /// Validate the remote ingress configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.urls.is_empty() {
            return Err("Remote ingress must have at least one URL".to_string());
        }
        Ok(())
    }
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
        // Empty mesh is now valid - it simply won't match anything
        let mesh = Mesh::default();
        assert!(mesh.validate().is_ok());

        // Mesh with ingress only is valid
        let mesh_with_ingress = Mesh {
            ingress: vec!["ingress1".to_string()],
            ..Default::default()
        };
        assert!(mesh_with_ingress.validate().is_ok());

        // Mesh with both ingress and egress is valid
        let mesh_with_both = Mesh {
            ingress: vec!["ingress1".to_string()],
            egress: vec!["egress1".to_string()],
            ..Default::default()
        };
        assert!(mesh_with_both.validate().is_ok());
    }

    #[test]
    fn test_ingress_validation() {
        let mut ingress = MeshIngress::default();
        assert!(ingress.validate().is_err()); // missing pipeline

        ingress.pipeline = "my-pipeline".to_string();
        assert!(ingress.validate().is_err()); // missing urls

        ingress.urls.push("https://example.com".to_string());
        assert!(ingress.validate().is_ok());
    }

    #[test]
    fn test_egress_validation() {
        let mut egress = MeshEgress::default();
        assert!(egress.validate().is_err()); // missing pipeline

        egress.pipeline = "my-pipeline".to_string();
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
        // Default auth fields
        assert_eq!(mesh.auth_type, MeshAuthType::Jwt);
        assert!(mesh.jwt_secret.is_none());
    }

    #[test]
    fn test_mesh_auth_type_serialization() {
        let jwt = MeshAuthType::Jwt;
        assert_eq!(serde_json::to_string(&jwt).unwrap(), "\"jwt\"");
        assert_eq!(jwt.to_string(), "jwt");
    }

    #[test]
    fn test_mesh_deserialization_with_jwt_secret() {
        let toml_str = r#"
            type = "http"
            provider = "local"
            auth_type = "jwt"
            jwt_secret = "my-super-secret-key"
            ingress = ["ingress1"]
            egress = ["egress1"]
            enabled = true
        "#;

        let mesh: Mesh = toml::from_str(toml_str).unwrap();
        assert_eq!(mesh.mesh_type, MeshProtocol::Http);
        assert_eq!(mesh.provider, MeshProvider::Local);
        assert_eq!(mesh.auth_type, MeshAuthType::Jwt);
        assert_eq!(mesh.jwt_secret, Some("my-super-secret-key".to_string()));
        assert!(mesh.jwt_private_key_path.is_none());
        assert!(mesh.jwt_public_key_path.is_none());
    }

    #[test]
    fn test_mesh_deserialization_with_rsa_keys() {
        let toml_str = r#"
            type = "http"
            provider = "local"
            jwt_private_key_path = "/path/to/private.pem"
            jwt_public_key_path = "/path/to/public.pem"
            ingress = ["ingress1"]
            egress = ["egress1"]
        "#;

        let mesh: Mesh = toml::from_str(toml_str).unwrap();
        assert_eq!(mesh.jwt_private_key_path, Some("/path/to/private.pem".to_string()));
        assert_eq!(mesh.jwt_public_key_path, Some("/path/to/public.pem".to_string()));
        assert!(mesh.jwt_secret.is_none());
    }

    #[test]
    fn test_ingress_deserialization() {
        let toml_str = r#"
            pipeline = "fhir_pipeline"
            type = "http"
            endpoint = "api-endpoint"
            urls = ["https://api.example.com", "https://api2.example.com"]
        "#;

        let ingress: MeshIngress = toml::from_str(toml_str).unwrap();
        assert_eq!(ingress.pipeline, "fhir_pipeline");
        assert_eq!(ingress.ingress_type, MeshProtocol::Http);
        assert_eq!(ingress.endpoint, Some("api-endpoint".to_string()));
        assert_eq!(ingress.urls.len(), 2);
    }

    #[test]
    fn test_egress_deserialization() {
        let toml_str = r#"
            pipeline = "outbound_pipeline"
            type = "http3"
            backend = "remote-backend"
            description = "Egress to remote service"
        "#;

        let egress: MeshEgress = toml::from_str(toml_str).unwrap();
        assert_eq!(egress.pipeline, "outbound_pipeline");
        assert_eq!(egress.egress_type, MeshProtocol::Http3);
        assert_eq!(egress.backend, Some("remote-backend".to_string()));
        assert_eq!(egress.description, Some("Egress to remote service".to_string()));
    }
}
