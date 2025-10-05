use serde::{Deserialize, Serialize};
use axum::{
    http::{HeaderValue},
};
use base64::{engine::general_purpose, Engine as _};
use std::sync::Arc;
use std::error::Error as StdError;
use crate::models::envelope::envelope::Envelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct AuthSidecarConfig {
    pub token_path: String, // Existing field
    pub username: String,   // New field for Basic Auth username
    pub password: String,   // New field for Basic Auth password
}

pub struct AuthSidecarMiddleware {
    _config: Arc<AuthSidecarConfig>, // Reference to shared config (unused for now)
}

impl AuthSidecarMiddleware {
    pub fn new(config: AuthSidecarConfig) -> Self {
        Self {
            _config: Arc::new(config),
        }
    }
}

#[async_trait::async_trait]
impl Middleware for AuthSidecarMiddleware {
    async fn left(
        &self,
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        tracing::info!("Processing auth middleware (left)");

        // Extract Authorization header (case-insensitive; keys stored lowercase)
        let auth_header_opt = envelope
            .request_details
            .headers
            .get("authorization")
            .cloned();

        let header_val = match auth_header_opt {
            Some(h) => h,
            None => return Err("Missing Authorization header".into()),
        };

        // Validate Basic auth against configured username/password
        if !header_val.starts_with("Basic ") {
            return Err("Authorization header must start with 'Basic '".into());
        }
        let encoded = &header_val[6..];
        let decoded_bytes = general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| Error::from("Failed to decode Basic Auth credentials"))?;
        let decoded = String::from_utf8(decoded_bytes)
            .map_err(|_| Error::from("Failed to parse Basic Auth credentials as UTF-8"))?;

        let mut parts = decoded.splitn(2, ':');
        let user = parts.next().ok_or_else(|| Error::from("Missing username in Basic Auth credentials"))?;
        let pass = parts.next().ok_or_else(|| Error::from("Missing password in Basic Auth credentials"))?;

        if user == self._config.username && pass == self._config.password {
            Ok(envelope)
        } else {
            Err("Invalid username or password".into())
        }
    }

    async fn right(
        &self,
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        tracing::info!("Processing auth middleware (right)");
        Ok(envelope)
    }
}

#[allow(dead_code)]
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