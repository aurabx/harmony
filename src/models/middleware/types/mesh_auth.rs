//! MeshAuth Middleware
//!
//! Handles JWT-based authentication for mesh communication between Harmony proxies.
//!
//! - **Egress (left)**: Generates and attaches a JWT to outgoing requests to other mesh members
//! - **Ingress (right)**: Validates incoming JWTs from other mesh members
//!
//! The middleware is automatically injected by the PipelineExecutor when a request
//! flows through a mesh context. It does not need to be explicitly listed in the
//! pipeline middleware configuration.

use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::mesh::config::{MeshAuthType, MeshProvider};
use crate::models::middleware::middleware::Middleware;
use crate::models::middleware::AuthFailure;
use crate::utils::Error;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for MeshAuth middleware
#[derive(Clone)]
pub struct MeshAuthConfig {
    /// Mesh provider type
    pub provider: MeshProvider,
    /// Authentication type (currently only JWT)
    pub auth_type: MeshAuthType,
    /// Name of the mesh
    pub mesh_name: String,
    /// JWT secret for HS256 (local provider)
    pub jwt_secret: Option<String>,
    /// Encoding key for signing JWTs
    pub encoding_key: Option<Arc<EncodingKey>>,
    /// Decoding key for verifying JWTs
    pub decoding_key: Option<Arc<DecodingKey>>,
    /// Algorithm to use
    pub algorithm: Algorithm,
    /// Whether this is for ingress (validation) or egress (generation)
    pub direction: MeshAuthDirection,
}

impl std::fmt::Debug for MeshAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshAuthConfig")
            .field("provider", &self.provider)
            .field("auth_type", &self.auth_type)
            .field("mesh_name", &self.mesh_name)
            .field("jwt_secret", &self.jwt_secret.as_ref().map(|_| "[REDACTED]"))
            .field("encoding_key", &self.encoding_key.as_ref().map(|_| "[KEY]"))
            .field("decoding_key", &self.decoding_key.as_ref().map(|_| "[KEY]"))
            .field("algorithm", &self.algorithm)
            .field("direction", &self.direction)
            .finish()
    }
}

/// Direction of mesh auth - determines whether we generate or validate JWTs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshAuthDirection {
    /// Egress: Generate and attach JWT to outgoing requests
    Egress,
    /// Ingress: Validate incoming JWT from other mesh members
    Ingress,
}

/// JWT claims for mesh authentication
#[derive(Debug, Serialize, Deserialize)]
pub struct MeshClaims {
    /// Subject - source proxy/service identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// Issuer - mesh name or proxy identifier
    pub iss: String,
    /// Audience - target mesh member (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    /// Issued at timestamp
    pub iat: i64,
    /// Expiration timestamp
    pub exp: i64,
    /// Mesh identifier
    pub mesh_id: String,
}

/// MeshAuth middleware for JWT-based mesh authentication
#[derive(Debug)]
pub struct MeshAuthMiddleware {
    config: MeshAuthConfig,
}

impl MeshAuthMiddleware {
    /// Create a new MeshAuthMiddleware with the given configuration
    pub fn new(config: MeshAuthConfig) -> Self {
        Self { config }
    }

    /// Create MeshAuthMiddleware for egress (JWT generation)
    pub fn for_egress(
        mesh_name: String,
        provider: MeshProvider,
        jwt_secret: Option<String>,
        jwt_private_key_path: Option<String>,
    ) -> Result<Self, String> {
        let (encoding_key, decoding_key, algorithm) = match provider {
            MeshProvider::Local => {
                if let Some(ref secret) = jwt_secret {
                    // Use HS256 with shared secret
                    let encoding = EncodingKey::from_secret(secret.as_bytes());
                    let decoding = DecodingKey::from_secret(secret.as_bytes());
                    (Some(Arc::new(encoding)), Some(Arc::new(decoding)), Algorithm::HS256)
                } else if let Some(ref key_path) = jwt_private_key_path {
                    // Use RS256 with private key for signing
                    let pem = std::fs::read_to_string(key_path)
                        .map_err(|e| format!("Failed to read JWT private key at '{}': {}", key_path, e))?;
                    let encoding = EncodingKey::from_rsa_pem(pem.as_bytes())
                        .map_err(|e| format!("Failed to parse RSA private key: {}", e))?;
                    (Some(Arc::new(encoding)), None, Algorithm::RS256)
                } else {
                    return Err("Local mesh provider requires jwt_secret or jwt_private_key_path".to_string());
                }
            }
            MeshProvider::Runbeam => {
                // Runbeam provider - JWT will be fetched from API (stubbed)
                // No local keys needed for egress
                (None, None, Algorithm::HS256)
            }
        };

        Ok(Self {
            config: MeshAuthConfig {
                provider,
                auth_type: MeshAuthType::Jwt,
                mesh_name,
                jwt_secret,
                encoding_key,
                decoding_key,
                algorithm,
                direction: MeshAuthDirection::Egress,
            },
        })
    }

    /// Create MeshAuthMiddleware for ingress (JWT validation)
    pub fn for_ingress(
        mesh_name: String,
        provider: MeshProvider,
        jwt_secret: Option<String>,
        jwt_public_key_path: Option<String>,
    ) -> Result<Self, String> {
        let (encoding_key, decoding_key, algorithm) = match provider {
            MeshProvider::Local => {
                if let Some(ref secret) = jwt_secret {
                    // Use HS256 with shared secret
                    let encoding = EncodingKey::from_secret(secret.as_bytes());
                    let decoding = DecodingKey::from_secret(secret.as_bytes());
                    (Some(Arc::new(encoding)), Some(Arc::new(decoding)), Algorithm::HS256)
                } else if let Some(ref key_path) = jwt_public_key_path {
                    // Use RS256 with public key for verification
                    let pem = std::fs::read_to_string(key_path)
                        .map_err(|e| format!("Failed to read JWT public key at '{}': {}", key_path, e))?;
                    let decoding = DecodingKey::from_rsa_pem(pem.as_bytes())
                        .map_err(|e| format!("Failed to parse RSA public key: {}", e))?;
                    (None, Some(Arc::new(decoding)), Algorithm::RS256)
                } else {
                    return Err("Local mesh provider requires jwt_secret or jwt_public_key_path".to_string());
                }
            }
            MeshProvider::Runbeam => {
                // Runbeam provider - JWT will be verified via API (stubbed)
                // No local keys needed for ingress
                (None, None, Algorithm::HS256)
            }
        };

        Ok(Self {
            config: MeshAuthConfig {
                provider,
                auth_type: MeshAuthType::Jwt,
                mesh_name,
                jwt_secret,
                encoding_key,
                decoding_key,
                algorithm,
                direction: MeshAuthDirection::Ingress,
            },
        })
    }

    /// Generate a JWT for egress requests
    fn generate_jwt(&self) -> Result<String, Error> {
        match self.config.provider {
            MeshProvider::Local => self.generate_local_jwt(),
            MeshProvider::Runbeam => self.fetch_runbeam_jwt(),
        }
    }

    /// Generate JWT locally using configured key
    fn generate_local_jwt(&self) -> Result<String, Error> {
        let encoding_key = self.config.encoding_key.as_ref()
            .ok_or_else(|| Error::from("No encoding key configured for local mesh JWT generation"))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::from(format!("System time error: {}", e)))?
            .as_secs() as i64;

        let claims = MeshClaims {
            sub: Some("harmony-proxy".to_string()),
            iss: self.config.mesh_name.clone(),
            aud: None,
            iat: now,
            exp: now + 300, // 5 minute expiry
            mesh_id: self.config.mesh_name.clone(),
        };

        let header = Header::new(self.config.algorithm);
        encode(&header, &claims, encoding_key)
            .map_err(|e| Error::from(format!("Failed to generate JWT: {}", e)))
    }

    /// Fetch JWT from Runbeam API (stubbed)
    fn fetch_runbeam_jwt(&self) -> Result<String, Error> {
        // TODO: Implement actual Runbeam API call
        // For now, return a placeholder that indicates this is a stubbed response
        tracing::warn!(
            "Runbeam mesh JWT fetch is stubbed - mesh: {}",
            self.config.mesh_name
        );
        
        // Return a stub JWT (this won't validate but allows the flow to continue for testing)
        Ok("runbeam-stub-jwt-token".to_string())
    }

    /// Validate an incoming JWT
    fn validate_jwt(&self, token: &str) -> Result<MeshClaims, Error> {
        match self.config.provider {
            MeshProvider::Local => self.validate_local_jwt(token),
            MeshProvider::Runbeam => self.validate_runbeam_jwt(token),
        }
    }

    /// Validate JWT locally using configured key
    fn validate_local_jwt(&self, token: &str) -> Result<MeshClaims, Error> {
        let decoding_key = self.config.decoding_key.as_ref()
            .ok_or_else(|| AuthFailure("No decoding key configured for local mesh JWT validation"))?;

        let mut validation = Validation::new(self.config.algorithm);
        validation.validate_exp = true;
        validation.leeway = 60; // 60 second leeway

        let token_data = decode::<MeshClaims>(token, decoding_key, &validation)
            .map_err(|e| {
                tracing::warn!("Mesh JWT validation failed: {}", e);
                AuthFailure("Invalid mesh JWT")
            })?;

        // Verify mesh_id matches expected mesh
        if token_data.claims.mesh_id != self.config.mesh_name {
            tracing::warn!(
                "Mesh JWT mesh_id mismatch: expected '{}', got '{}'",
                self.config.mesh_name,
                token_data.claims.mesh_id
            );
            return Err(AuthFailure("Mesh JWT mesh_id mismatch").into());
        }

        Ok(token_data.claims)
    }

    /// Validate JWT via Runbeam API (stubbed)
    fn validate_runbeam_jwt(&self, token: &str) -> Result<MeshClaims, Error> {
        // TODO: Implement actual Runbeam API call for JWT validation
        tracing::warn!(
            "Runbeam mesh JWT validation is stubbed - mesh: {}",
            self.config.mesh_name
        );

        // For stubbed implementation, accept the stub token
        if token == "runbeam-stub-jwt-token" {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            return Ok(MeshClaims {
                sub: Some("stubbed-source".to_string()),
                iss: self.config.mesh_name.clone(),
                aud: None,
                iat: now,
                exp: now + 300,
                mesh_id: self.config.mesh_name.clone(),
            });
        }

        Err(AuthFailure("Runbeam JWT validation not implemented").into())
    }

    /// Extract JWT from Authorization header
    fn extract_token(&self, headers: &std::collections::HashMap<String, String>) -> Option<String> {
        headers
            .get("authorization")
            .or_else(|| headers.get("Authorization"))
            .and_then(|auth| {
                if auth.starts_with("Bearer ") {
                    Some(auth.trim_start_matches("Bearer ").to_string())
                } else {
                    None
                }
            })
    }
}

#[async_trait::async_trait]
impl Middleware for MeshAuthMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Only generate JWT for egress direction
        if self.config.direction != MeshAuthDirection::Egress {
            return Ok(envelope);
        }

        tracing::debug!(
            "MeshAuth egress: generating JWT for mesh '{}'",
            self.config.mesh_name
        );

        // Generate JWT
        let jwt = self.generate_jwt()?;

        // Add JWT to target_details headers (for backend request)
        envelope.set_target_header("Authorization", format!("Bearer {}", jwt));

        // Also add to metadata for visibility
        envelope.request_details.metadata.insert(
            "mesh_auth_token_attached".to_string(),
            "true".to_string(),
        );

        tracing::info!(
            "MeshAuth: attached JWT for egress to mesh '{}'",
            self.config.mesh_name
        );

        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        // Only validate JWT for ingress direction
        if self.config.direction != MeshAuthDirection::Ingress {
            return Ok(envelope);
        }

        tracing::debug!(
            "MeshAuth ingress: validating JWT for mesh '{}'",
            self.config.mesh_name
        );

        // Extract token from request headers
        let token = self.extract_token(&envelope.request_details.headers)
            .ok_or_else(|| AuthFailure("Missing mesh Authorization header"))?;

        // Validate the token
        let claims = self.validate_jwt(&token)?;

        // Add validated claims to response metadata
        envelope.response_details.metadata.insert(
            "mesh_auth_validated".to_string(),
            "true".to_string(),
        );
        if let Some(sub) = claims.sub {
            envelope.response_details.metadata.insert(
                "mesh_auth_subject".to_string(),
                sub,
            );
        }
        envelope.response_details.metadata.insert(
            "mesh_auth_issuer".to_string(),
            claims.iss,
        );

        tracing::info!(
            "MeshAuth: validated JWT for ingress from mesh '{}'",
            self.config.mesh_name
        );

        Ok(envelope)
    }
}

/// Parse MeshAuth configuration from middleware options
pub fn parse_config(
    options: &std::collections::HashMap<String, JsonValue>,
) -> Result<MeshAuthConfig, String> {
    parse_config_with_context(options, None)
}

/// Parse MeshAuth configuration with optional Config context for mesh inference
///
/// When `mesh_name` is not provided in options, attempts to infer it from Config:
/// - If exactly one mesh exists, uses that mesh's name and settings
/// - If multiple meshes exist, returns an error (ambiguous)
/// - If no meshes exist, returns an error
pub fn parse_config_with_context(
    options: &std::collections::HashMap<String, JsonValue>,
    config: Option<&crate::config::config::Config>,
) -> Result<MeshAuthConfig, String> {
    // Try to get mesh_name from options first
    let explicit_mesh_name = options
        .get("mesh_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // If no explicit mesh_name, try to infer from config
    let (mesh_name, inferred_mesh) = if let Some(name) = explicit_mesh_name {
        // Explicit mesh_name provided - look up the mesh config if available
        let mesh = config.and_then(|c| c.mesh.get(&name));
        (name, mesh)
    } else if let Some(cfg) = config {
        // Try to infer from config
        let enabled_meshes: Vec<_> = cfg.mesh.iter()
            .filter(|(_, m)| m.enabled)
            .collect();
        
        match enabled_meshes.len() {
            0 => return Err("mesh_name is required for mesh_auth middleware (no meshes configured)".to_string()),
            1 => {
                let (name, mesh) = enabled_meshes.into_iter().next().unwrap();
                tracing::debug!("Inferred mesh_name '{}' for mesh_auth middleware", name);
                (name.clone(), Some(mesh))
            }
            _ => return Err(format!(
                "mesh_name is required for mesh_auth middleware (multiple meshes configured: {})",
                enabled_meshes.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
            )),
        }
    } else {
        return Err("mesh_name is required for mesh_auth middleware".to_string());
    };

    // Get provider - prefer options, fall back to mesh config, default to local
    let provider_str = options
        .get("provider")
        .and_then(|v| v.as_str());
    
    let provider = if let Some(p) = provider_str {
        match p {
            "local" => MeshProvider::Local,
            "runbeam" => MeshProvider::Runbeam,
            _ => return Err(format!("Unknown mesh provider: {}", p)),
        }
    } else if let Some(mesh) = inferred_mesh {
        mesh.provider.clone()
    } else {
        MeshProvider::Local
    };

    let direction_str = options
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("ingress"); // Default to ingress for manually configured mesh_auth

    let direction = match direction_str {
        "egress" => MeshAuthDirection::Egress,
        "ingress" => MeshAuthDirection::Ingress,
        _ => return Err(format!("Unknown mesh auth direction: {}", direction_str)),
    };

    // Get JWT secret - prefer options, fall back to mesh config
    let jwt_secret = options
        .get("jwt_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| inferred_mesh.and_then(|m| m.jwt_secret.clone()));

    // Get JWT key path - prefer options, fall back to mesh config based on direction
    let jwt_key_path = options
        .get("jwt_key_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            inferred_mesh.and_then(|m| match direction {
                MeshAuthDirection::Egress => m.jwt_private_key_path.clone(),
                MeshAuthDirection::Ingress => m.jwt_public_key_path.clone(),
            })
        });

    // Build the middleware based on direction
    match direction {
        MeshAuthDirection::Egress => {
            let mw = MeshAuthMiddleware::for_egress(mesh_name, provider, jwt_secret, jwt_key_path)?;
            Ok(mw.config)
        }
        MeshAuthDirection::Ingress => {
            let mw = MeshAuthMiddleware::for_ingress(mesh_name, provider, jwt_secret, jwt_key_path)?;
            Ok(mw.config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{RequestDetails, RequestEnvelopeBuilder, ResponseDetails};
    use std::collections::HashMap;

    fn create_test_request_envelope() -> RequestEnvelope<serde_json::Value> {
        RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/test")
            .original_data(serde_json::json!({}))
            .normalized_data(Some(serde_json::json!({"test": "data"})))
            .build()
            .unwrap()
    }

    fn create_test_response_envelope(
        auth_header: Option<&str>,
    ) -> ResponseEnvelope<serde_json::Value> {
        let mut headers = HashMap::new();
        if let Some(auth) = auth_header {
            headers.insert("authorization".to_string(), auth.to_string());
        }

        ResponseEnvelope {
            request_details: RequestDetails {
                method: "GET".to_string(),
                uri: "/test".to_string(),
                headers,
                cookies: HashMap::new(),
                query_params: HashMap::new(),
                cache_status: None,
                metadata: HashMap::new(),
                content_metadata: None,
            },
            response_details: ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            original_data: serde_json::json!({}),
            normalized_data: Some(serde_json::json!({"response": "data"})),
            normalized_snapshot: None,
        }
    }

    #[test]
    fn test_mesh_auth_config_creation_local_hs256() {
        let result = MeshAuthMiddleware::for_egress(
            "test-mesh".to_string(),
            MeshProvider::Local,
            Some("test-secret".to_string()),
            None,
        );
        assert!(result.is_ok());
        let mw = result.unwrap();
        assert_eq!(mw.config.algorithm, Algorithm::HS256);
        assert!(mw.config.encoding_key.is_some());
    }

    #[test]
    fn test_mesh_auth_config_creation_local_no_secret() {
        let result = MeshAuthMiddleware::for_egress(
            "test-mesh".to_string(),
            MeshProvider::Local,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("jwt_secret or jwt_private_key_path"));
    }

    #[test]
    fn test_mesh_auth_config_creation_runbeam() {
        let result = MeshAuthMiddleware::for_egress(
            "test-mesh".to_string(),
            MeshProvider::Runbeam,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_egress_generates_jwt() {
        let mw = MeshAuthMiddleware::for_egress(
            "test-mesh".to_string(),
            MeshProvider::Local,
            Some("test-secret-key-for-jwt".to_string()),
            None,
        )
        .unwrap();

        let envelope = create_test_request_envelope();
        let result = mw.left(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();
        // Check that target_details has the Authorization header
        let target = processed.target_details.as_ref().unwrap();
        assert!(target.headers.contains_key("Authorization"));
        assert!(target.headers.get("Authorization").unwrap().starts_with("Bearer "));

        // Check metadata
        assert_eq!(
            processed.request_details.metadata.get("mesh_auth_token_attached"),
            Some(&"true".to_string())
        );
    }

    #[tokio::test]
    async fn test_ingress_validates_jwt() {
        let secret = "test-secret-key-for-jwt";

        // Generate a valid JWT first
        let egress_mw = MeshAuthMiddleware::for_egress(
            "test-mesh".to_string(),
            MeshProvider::Local,
            Some(secret.to_string()),
            None,
        )
        .unwrap();

        let jwt = egress_mw.generate_jwt().unwrap();

        // Now validate it with ingress middleware
        let ingress_mw = MeshAuthMiddleware::for_ingress(
            "test-mesh".to_string(),
            MeshProvider::Local,
            Some(secret.to_string()),
            None,
        )
        .unwrap();

        let envelope = create_test_response_envelope(Some(&format!("Bearer {}", jwt)));
        let result = ingress_mw.right(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();
        assert_eq!(
            processed.response_details.metadata.get("mesh_auth_validated"),
            Some(&"true".to_string())
        );
    }

    #[tokio::test]
    async fn test_ingress_rejects_invalid_jwt() {
        let ingress_mw = MeshAuthMiddleware::for_ingress(
            "test-mesh".to_string(),
            MeshProvider::Local,
            Some("test-secret".to_string()),
            None,
        )
        .unwrap();

        let envelope = create_test_response_envelope(Some("Bearer invalid-token"));
        let result = ingress_mw.right(envelope).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ingress_rejects_missing_auth() {
        let ingress_mw = MeshAuthMiddleware::for_ingress(
            "test-mesh".to_string(),
            MeshProvider::Local,
            Some("test-secret".to_string()),
            None,
        )
        .unwrap();

        let envelope = create_test_response_envelope(None);
        let result = ingress_mw.right(envelope).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_egress_passthrough_for_ingress_direction() {
        let mw = MeshAuthMiddleware::for_ingress(
            "test-mesh".to_string(),
            MeshProvider::Local,
            Some("test-secret".to_string()),
            None,
        )
        .unwrap();

        let envelope = create_test_request_envelope();
        let result = mw.left(envelope).await;
        assert!(result.is_ok());

        // Should pass through without modification
        let processed = result.unwrap();
        assert!(processed.target_details.is_none() || 
            !processed.target_details.as_ref().unwrap().headers.contains_key("Authorization"));
    }

    #[tokio::test]
    async fn test_runbeam_stub_jwt() {
        let egress_mw = MeshAuthMiddleware::for_egress(
            "test-mesh".to_string(),
            MeshProvider::Runbeam,
            None,
            None,
        )
        .unwrap();

        let jwt = egress_mw.generate_jwt().unwrap();
        assert_eq!(jwt, "runbeam-stub-jwt-token");
    }

    #[test]
    fn test_parse_config_egress() {
        let mut options = HashMap::new();
        options.insert("mesh_name".to_string(), serde_json::json!("my-mesh"));
        options.insert("provider".to_string(), serde_json::json!("local"));
        options.insert("direction".to_string(), serde_json::json!("egress"));
        options.insert("jwt_secret".to_string(), serde_json::json!("secret"));

        let result = parse_config(&options);
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.direction, MeshAuthDirection::Egress);
    }

    #[test]
    fn test_parse_config_ingress() {
        let mut options = HashMap::new();
        options.insert("mesh_name".to_string(), serde_json::json!("my-mesh"));
        options.insert("provider".to_string(), serde_json::json!("local"));
        options.insert("direction".to_string(), serde_json::json!("ingress"));
        options.insert("jwt_secret".to_string(), serde_json::json!("secret"));

        let result = parse_config(&options);
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.direction, MeshAuthDirection::Ingress);
    }

    #[test]
    fn test_parse_config_missing_mesh_name() {
        let options = HashMap::new();
        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mesh_name is required"));
    }
}
