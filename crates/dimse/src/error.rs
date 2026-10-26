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

    /// A failure with a known DIMSE status code. An SCP sends only `code` to
    /// its client. `detail` is for the server log: it often names hosts,
    /// ports and AE titles, so it must never go on the wire.
    #[error("DIMSE failure status 0x{code:04X}: {detail}")]
    Status { code: u16, detail: String },
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

    /// Create a failure that an SCP answers with `code`. `detail` is for
    /// the server log only.
    pub fn status(code: u16, detail: impl Into<String>) -> Self {
        Self::Status {
            code,
            detail: detail.into(),
        }
    }

    /// The DIMSE status code an SCP should send for this error, when the
    /// error carries one. Other errors have no code, and SCP handlers fall
    /// back to 0xC000.
    pub fn status_code(&self) -> Option<u16> {
        match self {
            DimseError::Status { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// Create a new internal error
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Create a new router error
    pub fn router(msg: impl Into<String>) -> Self {
        Self::Router(msg.into())
    }

    /// Create a new association error
    pub fn association(msg: impl Into<String>) -> Self {
        Self::DicomUl(msg.into())
    }

    /// Create a new network error
    pub fn network(msg: impl Into<String>) -> Self {
        Self::Network(std::io::Error::new(std::io::ErrorKind::Other, msg.into()))
    }

    /// Create a new parse error
    pub fn parse(msg: impl Into<String>) -> Self {
        Self::DicomParsing(msg.into())
    }

    /// True when the remote node could not be reached or would not hold an
    /// association: a network error, a failed or rejected association, or a
    /// timeout.
    ///
    /// False for errors from a node that answered, such as an unsupported SOP
    /// class or a failure status. Callers use this to decide whether a failed
    /// step means the node is unusable or only that one operation failed.
    pub fn is_connection_failure(&self) -> bool {
        matches!(
            self,
            DimseError::Network(_)
                | DimseError::DicomUl(_)
                | DimseError::AssociationRejected(_)
                | DimseError::Timeout(_)
        )
    }

    /// Check if this error is recoverable
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            DimseError::Network(_) | DimseError::Timeout(_) | DimseError::AssociationRejected(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_and_association_errors_are_connection_failures() {
        for error in [
            DimseError::network("connection refused"),
            DimseError::association("could not connect to server"),
            DimseError::AssociationRejected("called AE title not recognised".to_string()),
            DimseError::Timeout("no reply".to_string()),
        ] {
            assert!(error.is_connection_failure(), "{:?}", error);
        }
    }

    #[test]
    fn errors_from_a_node_that_answered_are_not_connection_failures() {
        for error in [
            DimseError::NotSupported("no presentation context for C-FIND".to_string()),
            DimseError::operation_failed("C-FIND failed with status: 0xC000"),
            DimseError::parse("bad identifier"),
        ] {
            assert!(!error.is_connection_failure(), "{:?}", error);
        }
    }

    #[test]
    fn status_error_carries_its_code() {
        let error = DimseError::status(0xA701, "PACS unreachable");
        assert_eq!(error.status_code(), Some(0xA701));
        assert_eq!(
            error.to_string(),
            "DIMSE failure status 0xA701: PACS unreachable"
        );
        assert!(!error.is_connection_failure());
    }

    #[test]
    fn errors_without_a_status_have_no_code() {
        for error in [
            DimseError::operation_failed("C-FIND failed with status: 0xC000"),
            DimseError::network("connection refused"),
            DimseError::internal("bug"),
        ] {
            assert_eq!(error.status_code(), None, "{:?}", error);
        }
    }
}
