use serde::{Deserialize, Serialize};
use std::fmt;

/// Runbeam API error type
///
/// Represents all possible errors that can occur when interacting with
/// the Runbeam Cloud API or performing related operations.
#[derive(Debug)]
pub enum RunbeamError {
    /// JWT validation failed
    JwtValidation(String),
    /// API request failed (network, HTTP, or response parsing error)
    Api(ApiError),
    /// Token storage operation failed
    Storage(crate::storage::StorageError),
    /// Configuration error
    Config(String),
}

impl fmt::Display for RunbeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunbeamError::JwtValidation(msg) => write!(f, "JWT validation failed: {}", msg),
            RunbeamError::Api(err) => write!(f, "API error: {}", err),
            RunbeamError::Storage(err) => write!(f, "Storage error: {}", err),
            RunbeamError::Config(msg) => write!(f, "Configuration error: {}", msg),
        }
    }
}

impl std::error::Error for RunbeamError {}

impl From<ApiError> for RunbeamError {
    fn from(err: ApiError) -> Self {
        RunbeamError::Api(err)
    }
}

impl From<crate::storage::StorageError> for RunbeamError {
    fn from(err: crate::storage::StorageError) -> Self {
        RunbeamError::Storage(err)
    }
}

impl From<jsonwebtoken::errors::Error> for RunbeamError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        RunbeamError::JwtValidation(err.to_string())
    }
}

/// API-specific errors
#[derive(Debug)]
pub enum ApiError {
    /// Network error (connection, timeout, etc.)
    Network(String),
    /// HTTP error with status code
    Http { status: u16, message: String },
    /// Failed to parse response
    Parse(String),
    /// Request building failed
    Request(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Network(msg) => write!(f, "Network error: {}", msg),
            ApiError::Http { status, message } => {
                write!(f, "HTTP {} error: {}", status, message)
            }
            ApiError::Parse(msg) => write!(f, "Parse error: {}", msg),
            ApiError::Request(msg) => write!(f, "Request error: {}", msg),
        }
    }
}

impl std::error::Error for ApiError {}

impl From<reqwest::Error> for ApiError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            ApiError::Network("Request timeout".to_string())
        } else if err.is_connect() {
            ApiError::Network(format!("Connection failed: {}", err))
        } else if let Some(status) = err.status() {
            ApiError::Http {
                status: status.as_u16(),
                message: err.to_string(),
            }
        } else {
            ApiError::Network(err.to_string())
        }
    }
}

/// User information from JWT claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub name: String,
}

/// Team information from JWT claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamInfo {
    pub id: String,
    pub name: String,
}

/// Gateway information returned from authorize endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayInfo {
    pub id: String,
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub authorized_by: Option<AuthorizedBy>,
}

/// User who authorized the gateway
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizedBy {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// Response from Runbeam Cloud authorize endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizeResponse {
    pub machine_token: String,
    pub expires_in: f64,
    pub expires_at: String,
    pub gateway: GatewayInfo,
    #[serde(default)]
    pub abilities: Vec<String>,
}
