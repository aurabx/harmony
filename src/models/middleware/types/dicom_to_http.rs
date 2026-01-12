//! Middleware that bridges DICOM DIMSE requests to HTTP streaming requests.
//!
//! This middleware transforms DIMSE operations (C-FIND, C-GET, C-MOVE) into HTTP POST
//! requests suitable for forwarding to an HTTP streaming backend.
//!
//! LEFT side: Converts DIMSE envelope to HTTP POST request
//! RIGHT side: Passes through HTTP response (streaming backend handles parsing)

use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use serde_json::Value;
use tracing::debug;

/// Middleware that bridges DICOM DIMSE requests to HTTP POST requests.
///
/// This is used when a DIMSE SCP needs to forward requests to an HTTP streaming
/// backend (e.g., another Harmony instance).
#[derive(Default, Debug)]
pub struct DicomToHttpMiddleware;

impl DicomToHttpMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Middleware for DicomToHttpMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<Value>, Error> {
        // Check if this is a DIMSE request by looking for dimse_op metadata
        let dimse_op = match envelope.request_details.metadata.get("dimse_op") {
            Some(op) => op.clone(),
            None => {
                // Not a DIMSE request, pass through unchanged
                return Ok(envelope);
            }
        };

        debug!(
            "dicom_to_http: transforming DIMSE operation '{}' to HTTP POST",
            dimse_op
        );

        // Transform to HTTP POST request
        envelope.request_details.method = "POST".to_string();

        // Set content type to JSON
        envelope
            .request_details
            .headers
            .insert("content-type".to_string(), "application/json".to_string());

        // Request streaming response format
        envelope
            .request_details
            .headers
            .insert("accept".to_string(), "application/x-ndjson".to_string());

        // Forward dimse_op as HTTP header so it can be received by the other Harmony instance
        envelope
            .request_details
            .headers
            .insert("x-dimse-op".to_string(), dimse_op.clone());

        // Forward query_level as HTTP header for C-FIND/C-GET operations
        if let Some(query_level) = envelope.request_details.metadata.get("query_level") {
            envelope
                .request_details
                .headers
                .insert("x-query-level".to_string(), query_level.clone());
        }

        // Update target details if present
        envelope.set_target_method("POST");

        // Ensure the dimse_op is preserved in metadata for the receiving end
        // (already there from the original request)

        debug!(
            "dicom_to_http: transformed request - method={}, headers={:?}",
            envelope.request_details.method, envelope.request_details.headers
        );

        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<Value>,
    ) -> Result<ResponseEnvelope<Value>, Error> {
        // Pass through response unchanged - the streaming backend handles parsing
        debug!(
            "dicom_to_http: passing through response with status {}",
            envelope.response_details.status
        );
        Ok(envelope)
    }
}
