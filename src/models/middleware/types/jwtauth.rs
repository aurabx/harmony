use jsonwebtoken::{DecodingKey, Algorithm, Validation, decode, decode_header};
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
};
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use serde_json::Value as JsonValue;

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct JwtAuthConfig {
    pub public_key_path: String, // Path to the public key file (RS256) or unused for HS256
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub audience: Option<String>,
    #[serde(default)]
    pub leeway_secs: Option<u64>,
    #[serde(default)]
    pub use_hs256: bool,               // When true, use HS256 with hs256_secret
    #[serde(default)]
    pub hs256_secret: Option<String>,  // Required if use_hs256 = true
}

pub struct JwtAuthMiddleware {
    pub config: JwtAuthConfig,              // Configuration for the middleware
    pub decoding_key: Arc<DecodingKey>,    // Decoding key for JWT
    pub algorithm: Algorithm,              // Expected algorithm
}

#[derive(Debug, Deserialize)]
struct Claims {
    #[allow(dead_code)]
    sub: Option<String>,
    #[allow(dead_code)]
    exp: Option<i64>,
    #[allow(dead_code)]
    nbf: Option<i64>,
    #[allow(dead_code)]
    iat: Option<i64>,
    #[allow(dead_code)]
    iss: Option<String>,
    #[allow(dead_code)]
    aud: Option<JsonValue>, // string or array
}

pub fn parse_config(options: &std::collections::HashMap<String, JsonValue>) -> Result<JwtAuthConfig, String> {
    let public_key_path = options
        .get("public_key_path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let issuer = options.get("issuer").and_then(|v| v.as_str()).map(|s| s.to_string());
    let audience = options.get("audience").and_then(|v| v.as_str()).map(|s| s.to_string());
    let leeway_secs = options.get("leeway_secs").and_then(|v| v.as_u64());
    let use_hs256 = options
        .get("use_hs256")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let hs256_secret = options
        .get("hs256_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(JwtAuthConfig {
        public_key_path,
        issuer,
        audience,
        leeway_secs,
        use_hs256,
        hs256_secret,
    })
}

impl JwtAuthMiddleware {
    pub fn new(config: JwtAuthConfig) -> Self {
        // Try to load an RSA public key; if unavailable, fall back to a local secret for tests/dev.
        // Choose algorithm based on config
        let (decoding_key, algorithm) = if config.use_hs256 {
            let secret = config
                .hs256_secret
                .as_ref()
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_else(|| b"test-fallback-secret".to_vec());
            (DecodingKey::from_secret(&secret), Algorithm::HS256)
        } else {
            // Strict RS256 path: must load a valid RSA public key
            match std::fs::read_to_string(&config.public_key_path) {
                Ok(pem) => {
                    match DecodingKey::from_rsa_pem(pem.as_bytes()) {
                        Ok(k) => (k, Algorithm::RS256),
                        Err(e) => {
                            panic!(
                                "JWT: failed to parse RSA public key at '{}': {}. Set use_hs256=true with a secret for HS256.",
                                config.public_key_path,
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    panic!(
                        "JWT: failed to read RSA public key at '{}': {}. Set use_hs256=true with a secret for HS256.",
                        config.public_key_path,
                        e
                    );
                }
            }
        };

        Self {
            config,
            decoding_key: Arc::new(decoding_key), // Use Arc to avoid reloading
            algorithm,
        }
    }

    /// Real token validation: verify signature and claims
    async fn validate_token(&self, token: &str) -> Result<bool, Error> {
        // Enforce expected algorithm from header
        let header = decode_header(token).map_err(|_| Error::from("invalid JWT header"))?;
        if header.alg != self.algorithm {
            return Err("unexpected JWT alg".into());
        }

        let mut validation = Validation::new(self.algorithm);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.leeway = self.config.leeway_secs.unwrap_or(60);

        if let Some(ref iss) = self.config.issuer {
            validation.set_issuer(&[iss.clone()]);
        }
        if let Some(ref aud) = self.config.audience {
            validation.set_audience(&[aud.clone()]);
        }

        let token_data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|_| Error::from("jwt verify failed"))?;

        // Additional audience check if aud is array or string
        if let (Some(expected), Some(aud_val)) = (&self.config.audience, &token_data.claims.aud) {
            match aud_val {
                JsonValue::String(s) if s == expected => {}
                JsonValue::Array(arr) if arr.iter().any(|v| v == expected) => {}
                _ => return Err("aud mismatch".into()),
            }
        }

        Ok(true)
    }

    /// Extract JWT token from Authorization header in the envelope
    fn extract_token_from_envelope(&self, envelope: &RequestEnvelope<serde_json::Value>) -> Result<String, Error> {
        if let Some(auth_header) = envelope.request_details.headers.get("authorization") {
            if auth_header.starts_with("Bearer ") {
                Ok(auth_header.trim_start_matches("Bearer ").to_string())
            } else {
                Err("Authorization header must start with 'Bearer '".into())
            }
        } else {
            Err("Missing Authorization header".into())
        }
    }
}

#[async_trait::async_trait]
impl Middleware for JwtAuthMiddleware {
    async fn left(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Step 1: Extract the JWT token from the envelope's headers
        let token = match self.extract_token_from_envelope(&envelope) {
            Ok(token) => token,
            Err(err) => {
                tracing::error!("JWT Auth failed: {}", err);
                return Err(err);
            }
        };

        // Step 2: Validate the token
        if !self.validate_token(&token).await? {
            let error_message = "Invalid or expired JWT token";
            tracing::error!("{}", error_message);
            return Err(error_message.into());
        }

        tracing::info!("JWT token validated successfully");

        // Step 3: Pass through the envelope (token is valid)
        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // For JWT auth, typically no processing is needed on the right side
        // Just pass through the envelope
        tracing::info!("JWT Auth middleware processing response (right)");
        Ok(envelope)
    }
}