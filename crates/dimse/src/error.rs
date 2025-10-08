//! Error types for DIMSE operations

use thiserror::Error;

/// Result type alias for DIMSE operations
pub type Result<T> = std::result::Result<T, DimseError>;

/// Error types that can occur during DIMSE operations
#[derive(Error, Debug)]
pub enum DimseError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Network error: {0}")]
    Network(#[from] std::io::Error),

    #[error("DICOM parsing error: {0}")]
    DicomParsing(String),

    #[error("DICOM object error: {0}")]
    DicomObject(String),

    #[error("DICOM UL error: {0}")]
    DicomUl(String),

    #[error("Association rejected: {0}")]
    AssociationRejected(String),

    #[error("DIMSE operation failed: {0}")]
    OperationFailed(String),

    #[error("Invalid AE Title: {0}")]
    InvalidAeTitle(String),

    #[error("Timeout occurred: {0}")]
    Timeout(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Router error: {0}")]
    Router(String),

    #[cfg(feature = "tls")]
    #[error("TLS error: {0}")]
    Tls(#[from] tokio_rustls::rustls::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Operation not supported: {0}")]
    NotSupported(String),
}

impl DimseError {
    /// Create a new configuration error
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Create a new operation failed error
    pub fn operation_failed(msg: impl Into<String>) -> Self {
        Self::OperationFailed(msg.into())
    }

    /// Create a new internal error
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Create a new router error
    pub fn router(msg: impl Into<String>) -> Self {
        Self::Router(msg.into())
    }

    /// Check if this error is recoverable
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            DimseError::Network(_) | DimseError::Timeout(_) | DimseError::AssociationRejected(_)
        )
    }
}
