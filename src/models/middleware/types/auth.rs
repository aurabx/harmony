use serde::Deserialize;
use axum::{
    http::{HeaderValue},
};
use base64::{engine::general_purpose, Engine as _};
use std::sync::Arc;
use std::error::Error as StdError;
use crate::models::envelope::envelope::Envelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;

#[derive(Debug, Deserialize, Clone)]
pub struct AuthSidecarConfig {
    pub token_path: String, // Existing field
    pub username: String,   // New field for Basic Auth username
    pub password: String,   // New field for Basic Auth password
}

pub struct AuthSidecarMiddleware {
    config: Arc<AuthSidecarConfig>, // Reference to shared config
}

impl AuthSidecarMiddleware {
    pub fn new(config: AuthSidecarConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

#[async_trait::async_trait]
impl Middleware for AuthSidecarMiddleware {
    async fn left(
        &self,
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        // For now, just pass through the envelope
        // In a real implementation, you would:
        // 1. Extract auth information from envelope.request_details.headers
        // 2. Validate the auth credentials
        // 3. Either return the envelope or return an error

        tracing::info!("Processing auth middleware (left)");
        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        // For now, just pass through the envelope
        tracing::info!("Processing auth middleware (right)");
        Ok(envelope)
    }
}

async fn validate_basic_auth(
    header: &HeaderValue,
    config: &AuthSidecarConfig,
) -> Result<(), Box<dyn StdError + Send + Sync>> {
    // Ensure the header starts with "Basic "
    let header_str = header
        .to_str()
        .map_err(|_| "Invalid Authorization header format")?;
    if !header_str.starts_with("Basic ") {
        return Err("Authorization header must start with 'Basic '".into());
    }

    // Decode the base64-encoded credentials
    let encoded_credentials = &header_str[6..]; // Skip "Basic " prefix
    let decoded_bytes = general_purpose::STANDARD
        .decode(encoded_credentials)
        .map_err(|_| "Failed to decode Basic Auth credentials")?;
    let decoded_credentials = String::from_utf8(decoded_bytes)
        .map_err(|_| "Failed to parse Basic Auth credentials as UTF-8")?;

    // Split the credentials into username and password
    let mut parts = decoded_credentials.splitn(2, ':');
    let username = parts.next().ok_or("Missing username in Basic Auth credentials")?;
    let password = parts.next().ok_or("Missing password in Basic Auth credentials")?;

    // Validate the credentials against the config
    if username == config.username && password == config.password {
        Ok(())
    } else {
        Err("Invalid username or password".into())
    }
}