use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use std::{
    sync::Arc,
};
use crate::models::envelope::envelope::Envelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;

#[derive(Debug, Deserialize, Clone)]
pub struct JwtAuthConfig {
    pub public_key_path: String, // Path to the public key file
}

pub struct JwtAuthMiddleware {
    pub config: JwtAuthConfig,              // Configuration for the middleware
    pub decoding_key: Arc<DecodingKey>,    // Decoding key for JWT
}

#[derive(Debug, Deserialize)]
struct Claims {
    #[allow(dead_code)]
    sub: String, // Subject (user ID, etc.)
    #[allow(dead_code)]
    exp: usize,  // Expiration time (UNIX timestamp)
    #[allow(dead_code)]
    iss: String, // Issuer
    #[allow(dead_code)]
    aud: String, // Audience
}

impl JwtAuthMiddleware {
    pub fn new(config: JwtAuthConfig) -> Self {
        // Load the public key from the provided path
        let public_key = std::fs::read_to_string(&config.public_key_path)
            .expect("Failed to read the public key file for JWT validation");

        let decoding_key = DecodingKey::from_rsa_pem(public_key.as_bytes())
            .expect("Failed to parse the public key for JWT validation");

        Self {
            config,
            decoding_key: Arc::new(decoding_key), // Use Arc to avoid reloading
        }
    }

    /// Simulated token validation logic
    async fn validate_token(&self, token: &str) -> Result<bool, Error> {
        // Replace this stub with real token validation logic,
        // such as verifying a JWT against a public key, verifying the issuer/audience, etc.
        tracing::info!("Validating token: {}", token);
        // For demonstration purposes, assume tokens starting with "valid-" are accepted
        Ok(token.starts_with("valid-"))
    }

    /// Extract JWT token from Authorization header in the envelope
    fn extract_token_from_envelope(&self, envelope: &Envelope<serde_json::Value>) -> Result<String, Error> {
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
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
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
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        // For JWT auth, typically no processing is needed on the right side
        // Just pass through the envelope
        tracing::info!("JWT Auth middleware processing response (right)");
        Ok(envelope)
    }
}