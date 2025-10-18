//! DIMSE Status Code Mapping
//!
//! This module provides utilities to convert between HTTP status codes, pipeline errors,
//! and DICOM DIMSE status codes as defined in DICOM PS3.4.
//!
//! # Status Code Mapping
//!
//! DIMSE status codes follow these conventions:
//! - `0x0000`: Success
//! - `0x0124`: Not authorized
//! - `0xA801`: No such object instance (404-like)
//! - `0xA700-0xA702`: Resource limitations
//! - `0xC000`: Cannot understand (400-like)
//! - `0x0110`: Processing failure (500-like)
//! - `0xFE00`: Cancel
//! - `0xFF00`: Pending (C-FIND, C-MOVE)
//!
//! # Usage
//!
//! ```ignore
//! use crate::adapters::dimse::status_mapper;
//! use dimse::types::DimseStatus;
//!
//! // Convert HTTP status
//! let status = status_mapper::http_status_to_dimse(404);
//! assert_eq!(status, DimseStatus::Failure(0xA801));
//!
//! // Convert pipeline error
//! let error = PipelineError::ServiceError("Not found".to_string());
//! let status = status_mapper::pipeline_error_to_dimse(&error);
//! assert_eq!(status, DimseStatus::Failure(0xA801));
//!
//! // Check if status is retriable
//! if status_mapper::is_retriable_status(&status) {
//!     // Retry logic
//! }
//! ```

use crate::pipeline::executor::PipelineError;
use dimse::types::DimseStatus;

/// Maps HTTP status codes to DIMSE status codes
///
/// DICOM status codes follow PS3.4 conventions:
/// - 0x0000 = Success
/// - 0xFE00 = Cancel
/// - 0xFF00 = Pending (C-FIND, C-MOVE)
/// - 0xA700 = Out of Resources
/// - 0xA900 = Dataset does not match SOP Class
/// - 0xC000 = Cannot understand
/// - 0xC100-C1FF = Warning
/// - 0xFE00 = Processing failure
pub fn http_status_to_dimse(http_status: u16) -> DimseStatus {
    match http_status {
        // 2xx Success
        200..=299 => DimseStatus::Success,
        
        // 4xx Client Errors
        400 => DimseStatus::Failure(0xC000), // Cannot understand
        401 | 403 => DimseStatus::Failure(0x0124), // Not authorized
        404 => DimseStatus::Failure(0xA801), // No such object instance
        405 => DimseStatus::Failure(0x0111), // Duplicate invocation
        408 => DimseStatus::Failure(0x0122), // SOP class not supported
        409 => DimseStatus::Failure(0x0119), // Class-instance conflict
        410 => DimseStatus::Failure(0xA801), // No such object instance (gone)
        413 => DimseStatus::Failure(0xA700), // Out of resources
        415 => DimseStatus::Failure(0xA900), // Dataset does not match SOP class
        429 => DimseStatus::Failure(0xA702), // Resource limitation
        
        // 5xx Server Errors
        500 => DimseStatus::Failure(0x0110), // Processing failure
        501 => DimseStatus::Failure(0x0112), // Unrecognized operation
        502..=504 => DimseStatus::Failure(0xA701), // Out of resources/unable to process
        507 => DimseStatus::Failure(0xA700), // Out of resources
        
        // Default for unknown status codes
        _ if (400..500).contains(&http_status) => DimseStatus::Failure(0xC000),
        _ => DimseStatus::Failure(0x0110), // Processing failure
    }
}

/// Maps PipelineError to DIMSE status codes
pub fn pipeline_error_to_dimse(error: &PipelineError) -> DimseStatus {
    match error {
        PipelineError::ServiceError(msg) => {
            // Try to infer status from error message
            let msg_lower = msg.to_lowercase();
            if msg_lower.contains("not found") || msg_lower.contains("no such") {
                DimseStatus::Failure(0xA801) // No such object instance
            } else if msg_lower.contains("unauthorized") || msg_lower.contains("forbidden") {
                DimseStatus::Failure(0x0124) // Not authorized
            } else if msg_lower.contains("timeout") {
                DimseStatus::Failure(0x0122) // SOP class not supported (timeout)
            } else {
                DimseStatus::Failure(0x0110) // Processing failure
            }
        }
        
        PipelineError::MiddlewareError(_) => {
            DimseStatus::Failure(0x0110) // Processing failure
        }
        
        PipelineError::BackendError(msg) => {
            let msg_lower = msg.to_lowercase();
            if msg_lower.contains("not found") || msg_lower.contains("404") {
                DimseStatus::Failure(0xA801) // No such object instance
            } else if msg_lower.contains("timeout") {
                DimseStatus::Failure(0xA701) // Unable to process
            } else if msg_lower.contains("connection") || msg_lower.contains("network") {
                DimseStatus::Failure(0xA701) // Unable to process
            } else {
                DimseStatus::Failure(0x0110) // Processing failure
            }
        }
        
        PipelineError::ConfigError(_) => {
            DimseStatus::Failure(0x0110) // Processing failure
        }
    }
}

/// Maps common error scenarios to DIMSE status with context
pub fn error_context_to_dimse(http_status: Option<u16>, error: Option<&PipelineError>) -> DimseStatus {
    // Prefer pipeline error mapping if available (more specific)
    if let Some(err) = error {
        return pipeline_error_to_dimse(err);
    }
    
    // Fall back to HTTP status mapping
    if let Some(status) = http_status {
        return http_status_to_dimse(status);
    }
    
    // Default to generic processing failure
    DimseStatus::Failure(0x0110)
}

/// Check if a status indicates success (including warnings)
pub fn is_successful_status(status: &DimseStatus) -> bool {
    matches!(status, DimseStatus::Success | DimseStatus::Warning(_))
}

/// Check if a status indicates a retriable error
pub fn is_retriable_status(status: &DimseStatus) -> bool {
    match status {
        DimseStatus::Failure(code) => {
            matches!(
                *code,
                0xA701 | // Unable to process (temporary)
                0xA702 | // Resource limitation
                0xA700 | // Out of resources
                0x0122   // Timeout-related
            )
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_status_2xx_success() {
        assert_eq!(http_status_to_dimse(200), DimseStatus::Success);
        assert_eq!(http_status_to_dimse(201), DimseStatus::Success);
        assert_eq!(http_status_to_dimse(204), DimseStatus::Success);
    }

    #[test]
    fn test_http_status_404_not_found() {
        assert_eq!(http_status_to_dimse(404), DimseStatus::Failure(0xA801));
    }

    #[test]
    fn test_http_status_401_unauthorized() {
        assert_eq!(http_status_to_dimse(401), DimseStatus::Failure(0x0124));
    }

    #[test]
    fn test_http_status_500_server_error() {
        assert_eq!(http_status_to_dimse(500), DimseStatus::Failure(0x0110));
    }

    #[test]
    fn test_http_status_503_service_unavailable() {
        assert_eq!(http_status_to_dimse(503), DimseStatus::Failure(0xA701));
    }

    #[test]
    fn test_pipeline_error_not_found() {
        let error = PipelineError::ServiceError("Resource not found".to_string());
        assert_eq!(pipeline_error_to_dimse(&error), DimseStatus::Failure(0xA801));
    }

    #[test]
    fn test_pipeline_error_unauthorized() {
        let error = PipelineError::ServiceError("Unauthorized access".to_string());
        assert_eq!(pipeline_error_to_dimse(&error), DimseStatus::Failure(0x0124));
    }

    #[test]
    fn test_pipeline_error_generic() {
        let error = PipelineError::ServiceError("Something went wrong".to_string());
        assert_eq!(pipeline_error_to_dimse(&error), DimseStatus::Failure(0x0110));
    }

    #[test]
    fn test_backend_error_404() {
        let error = PipelineError::BackendError("Backend returned 404".to_string());
        assert_eq!(pipeline_error_to_dimse(&error), DimseStatus::Failure(0xA801));
    }

    #[test]
    fn test_error_context_prefers_pipeline_error() {
        let error = PipelineError::ServiceError("Not found".to_string());
        let status = error_context_to_dimse(Some(500), Some(&error));
        // Should use pipeline error mapping (0xA801) not HTTP status mapping (0x0110)
        assert_eq!(status, DimseStatus::Failure(0xA801));
    }

    #[test]
    fn test_error_context_falls_back_to_http() {
        let status = error_context_to_dimse(Some(404), None);
        assert_eq!(status, DimseStatus::Failure(0xA801));
    }

    #[test]
    fn test_error_context_default() {
        let status = error_context_to_dimse(None, None);
        assert_eq!(status, DimseStatus::Failure(0x0110));
    }

    #[test]
    fn test_is_successful_status() {
        assert!(is_successful_status(&DimseStatus::Success));
        assert!(is_successful_status(&DimseStatus::Warning(0xB000)));
        assert!(!is_successful_status(&DimseStatus::Failure(0x0110)));
        assert!(!is_successful_status(&DimseStatus::Pending));
    }

    #[test]
    fn test_is_retriable_status() {
        assert!(is_retriable_status(&DimseStatus::Failure(0xA701)));
        assert!(is_retriable_status(&DimseStatus::Failure(0xA702)));
        assert!(is_retriable_status(&DimseStatus::Failure(0xA700)));
        assert!(!is_retriable_status(&DimseStatus::Failure(0x0110)));
        assert!(!is_retriable_status(&DimseStatus::Success));
    }
}
