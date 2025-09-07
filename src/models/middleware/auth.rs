use serde::Deserialize;
use axum::{
    response::Response,
    http::{Request, HeaderValue, StatusCode},
    body::Body,
};
use tower::Service;
use base64::{engine::general_purpose, Engine as _};
use std::sync::Arc;
use std::error::Error as StdError;


use crate::models::middleware::{Middleware, Next, Error};

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
        request: Request<Body>,
        mut next: Next<Body>,
    ) -> Result<Response, Error> {
        // Step 1: Extract the `Authorization` header
        let header = match request.headers().get("Authorization") {
            Some(h) => h,
            None => {
                let error_message = "Missing Authorization header";
                tracing::error!("{}", error_message);
                return Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header("WWW-Authenticate", "Basic realm=\"example\"")
                    .body(Body::from(error_message))
                    .unwrap());
            }
        };

        // Step 2: Decode and validate the Basic Auth credentials
        match validate_basic_auth(header, &self.config).await {
            Ok(()) => {
                // Step 3: Pass the request to the next middleware in the middleware
                let response = next.call(request).await?;

                // Optionally log the response
                tracing::info!(
                    "Response status after AuthSidecarMiddleware: {}",
                    response.status()
                );

                Ok(response)
            }
            Err(err) => {
                tracing::error!("Invalid Basic Auth: {}", err);
                return Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header("WWW-Authenticate", "Basic realm=\"example\"")
                    .body(Body::from("Invalid Basic Auth credentials"))
                    .unwrap());
            }
        }
    }

    async fn right(&self, request: Request<Body>, next: Next<Body>) -> Result<Response, Error> {
        todo!()
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