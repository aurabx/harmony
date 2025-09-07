use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use axum::{
    body::Body,
    http::Request,
    response::Response,
    Error
};
use std::{
    error::Error as StdError,
    sync::Arc,
};
use tower::Service;
use crate::models::middleware::{Middleware, Next};

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
}


#[async_trait::async_trait]
impl Middleware for JwtAuthMiddleware {
    async fn left(
        &self,
        request: Request<Body>,
        mut next: Next<Body>,
    ) -> Result<Response, crate::models::middleware::types::Error> {
        // Step 1: Extract the "Authorization" header
        let token = match request.headers().get("Authorization") {
            Some(header_value) => {
                let header_str = header_value.to_str().map_err(|_| {
                    let error_message = "Invalid Authorization header format";
                    tracing::error!("{}", error_message);
                    Box::<dyn StdError + Send + Sync>::from(error_message)
                })?;
                if header_str.starts_with("Bearer ") {
                    header_str.trim_start_matches("Bearer ").to_string()
                } else {
                    let error_message = "Authorization header must start with 'Bearer '";
                    tracing::error!("{}", error_message);
                    return Ok(Response::builder()
                        .status(401)
                        .body(Body::from(error_message))
                        .unwrap());
                }
            }
            None => {
                let error_message = "Missing Authorization header";
                tracing::error!("{}", error_message);
                return Ok(Response::builder()
                    .status(401)
                    .body(Body::from(error_message))
                    .unwrap());
            }
        };

        // Step 2: Validate the token using your custom logic
        if !self.validate_token(&token).await? {
            let error_message = "Invalid or expired token";
            tracing::error!("{}", error_message);
            return Ok(Response::builder()
                .status(401)
                .body(Body::from(error_message))
                .unwrap());
        }

        // Step 3: Pass the request to the next middleware or handler
        let response = next.call(request).await?;

        // Step 4: Optionally modify the response (if needed)
        tracing::info!(
            "Response status after AuthSidecarMiddleware: {}",
            response.status()
        );

        Ok(response)
    }

    async fn right(&self, request: Request<Body>, next: Next<Body>) -> Result<Response, crate::models::middleware::types::Error> {
        todo!()
    }
}
