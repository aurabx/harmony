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
        // Only requests that arrived over DIMSE are bridged. HTTP-origin
        // requests can also carry a Command, so the protocol is checked too.
        let is_dimse = envelope
            .request_details
            .metadata
            .get("protocol")
            .is_some_and(|p| p == "dimse");
        let dimse_op = match envelope.resolve_operation().and_then(|op| op.dimse_op()) {
            Some(op) if is_dimse => op.to_string(),
            _ => return Ok(envelope),
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

        // Forward the operation as an HTTP header so the other Harmony instance can read it
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::command::{Command, Operation};
    use crate::models::envelope::envelope::RequestEnvelopeBuilder;
    use std::collections::HashMap;

    fn envelope(protocol: Option<&str>, command: Option<Command>) -> RequestEnvelope<Value> {
        let mut metadata = HashMap::new();
        if let Some(protocol) = protocol {
            metadata.insert("protocol".to_string(), protocol.to_string());
        }
        metadata.insert("query_level".to_string(), "STUDY".to_string());
        RequestEnvelopeBuilder::new()
            .method("find")
            .uri("dimse://scp/find")
            .headers(HashMap::new())
            .cookies(HashMap::new())
            .query_params(HashMap::new())
            .cache_status(None)
            .metadata(metadata)
            .command(command)
            .original_data(Value::Null)
            .build()
            .expect("build envelope")
    }

    #[tokio::test]
    async fn dimse_request_becomes_http_post_with_operation_header() {
        let env = envelope(Some("dimse"), Some(Command::new(Operation::Query)));

        let out = DicomToHttpMiddleware::new().left(env).await.expect("left");

        assert_eq!(out.request_details.method, "POST");
        let headers = &out.request_details.headers;
        assert_eq!(headers.get("x-dimse-op").map(String::as_str), Some("find"));
        assert_eq!(headers.get("x-query-level").map(String::as_str), Some("STUDY"));
        assert_eq!(headers.get("accept").map(String::as_str), Some("application/x-ndjson"));
    }

    #[tokio::test]
    async fn http_request_with_a_command_passes_through() {
        // FHIR and DICOMweb endpoints also set a Command. Only DIMSE-origin
        // requests may be rewritten.
        let env = envelope(Some("http"), Some(Command::new(Operation::Query)));

        let out = DicomToHttpMiddleware::new().left(env).await.expect("left");

        assert_eq!(out.request_details.method, "find");
        assert!(!out.request_details.headers.contains_key("x-dimse-op"));
    }

    #[tokio::test]
    async fn dimse_request_without_a_dimse_operation_passes_through() {
        for command in [None, Some(Command::new(Operation::Delete))] {
            let env = envelope(Some("dimse"), command);

            let out = DicomToHttpMiddleware::new().left(env).await.expect("left");

            assert_eq!(out.request_details.method, "find");
            assert!(!out.request_details.headers.contains_key("x-dimse-op"));
        }
    }
}
