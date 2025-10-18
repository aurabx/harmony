use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::models::middleware::AuthFailure;
use crate::utils::Error;
use axum::http::HeaderValue;
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::error::Error as StdError;
use std::sync::Arc;

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct AuthSidecarConfig {
    pub token_path: String, // Existing field
    pub username: String,   // New field for Basic Auth username
    pub password: String,   // New field for Basic Auth password
}

pub struct AuthSidecarMiddleware {
    _config: Arc<AuthSidecarConfig>, // Reference to shared config (unused for now)
}

pub fn parse_config(
    options: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<AuthSidecarConfig, String> {
    let token_path = options
        .get("token_path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let username = options
        .get("username")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'username' for auth middleware")?;

    let password = options
        .get("password")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'password' for auth middleware")?;

    Ok(AuthSidecarConfig {
        token_path,
        username: username.to_string(),
        password: password.to_string(),
    })
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
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        tracing::info!("Processing auth middleware (left)");

        // Extract Authorization header (case-insensitive; keys stored lowercase)
        let auth_header_opt = envelope
            .request_details
            .headers
            .get("authorization")
            .cloned();

        let header_val = match auth_header_opt {
            Some(h) => h,
            None => return Err(AuthFailure("Missing Authorization header").into()),
        };

        // Validate Basic auth against configured username/password
        if !header_val.starts_with("Basic ") {
            return Err(AuthFailure("Authorization header must start with 'Basic '").into());
        }
        let encoded = &header_val[6..];
        let decoded_bytes = general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| AuthFailure("Failed to decode Basic Auth credentials"))?;
        let decoded = String::from_utf8(decoded_bytes)
            .map_err(|_| AuthFailure("Failed to parse Basic Auth credentials as UTF-8"))?;

        let mut parts = decoded.splitn(2, ':');
        let user = parts
            .next()
            .ok_or(AuthFailure("Missing username in Basic Auth credentials"))?;
        let pass = parts
            .next()
            .ok_or(AuthFailure("Missing password in Basic Auth credentials"))?;

        if user == self._config.username && pass == self._config.password {
            Ok(envelope)
        } else {
            Err(AuthFailure("Invalid username or password").into())
        }
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        tracing::debug!("Processing auth middleware (right) - passthrough");
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
    let username = parts
        .next()
        .ok_or("Missing username in Basic Auth credentials")?;
    let password = parts
        .next()
        .ok_or("Missing password in Basic Auth credentials")?;

    // Validate the credentials against the config
    if username == config.username && password == config.password {
        Ok(())
    } else {
        Err("Invalid username or password".into())
    }
}
