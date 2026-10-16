//! Middleware that bridges DICOM DIMSE requests to HTTP streaming requests.
//!
//! This middleware transforms DIMSE operations (C-FIND, C-GET, C-MOVE, C-STORE) into
//! HTTP POST requests suitable for forwarding to an HTTP streaming backend.
//!
//! LEFT side: Converts DIMSE envelope to HTTP POST request. A C-STORE on its
//! way to an `http_streaming_backend` is sent as the DICOM Part 10 file itself.
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

    /// Make the stored instance the HTTP request body.
    ///
    /// The DIMSE adapter puts the Part 10 file in `part10_b64` when this
    /// middleware sits in front of an `http_streaming_backend`. Writing it to
    /// `original_data` as a lone
    /// `body_b64` makes the executor send the raw bytes, so the receiving
    /// host gets the instance itself and nothing that refers to this host's
    /// disk.
    ///
    /// A body without `part10_b64` is left as it is. The pipeline then has
    /// the `http` backend, which gets the DICOM JSON as before.
    fn send_instance_as_body(envelope: &mut RequestEnvelope<Value>) {
        let body = envelope
            .normalized_data
            .as_ref()
            .unwrap_or(&envelope.original_data);
        let Some(part10_b64) = body.get("part10_b64").and_then(Value::as_str) else {
            return;
        };
        let instance_body = serde_json::json!({ "body_b64": part10_b64 });

        let transfer_syntax = body
            .get("transfer_syntax_uid")
            .and_then(Value::as_str)
            .map(str::to_string);
        let sop_class_uid = body
            .pointer("/command/sop_class_uid")
            .and_then(Value::as_str)
            .map(str::to_string);
        let sop_instance_uid = envelope
            .request_details
            .metadata
            .get("SOPInstanceUID")
            .cloned();

        envelope.original_data = instance_body;

        let headers = &mut envelope.request_details.headers;
        headers.insert("content-type".to_string(), "application/dicom".to_string());
        for (name, value) in [
            ("x-dicom-transfer-syntax", transfer_syntax),
            ("x-dicom-sop-class-uid", sop_class_uid),
            ("x-dicom-sop-instance-uid", sop_instance_uid),
        ] {
            if let Some(value) = value {
                headers.insert(name.to_string(), value);
            }
        }
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

        if dimse_op == "store" {
            Self::send_instance_as_body(&mut envelope);
        }

        // Update target details if present
        envelope.set_target_method("POST");

        debug!(
            "dicom_to_http: transformed request - method={}, header names={:?}",
            envelope.request_details.method,
            crate::logging::sorted_keys(&envelope.request_details.headers)
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
    async fn debug_log_shows_header_names_without_values() {
        let marker = "SECRET-PHI-MARKER";
        let mut env = envelope(Some("dimse"), Some(Command::new(Operation::Query)));
        env.request_details
            .headers
            .insert("authorization".to_string(), format!("Bearer {marker}"));
        env.request_details
            .headers
            .insert("x-patient-name".to_string(), marker.to_string());

        let (log, _guard) = crate::logging::test_capture::capture_debug();
        DicomToHttpMiddleware::new().left(env).await.expect("left");

        let out = log.contents();
        assert!(
            out.contains("authorization"),
            "header names should be logged: {out}"
        );
        assert!(
            !out.contains(marker),
            "header values leaked into log: {out}"
        );
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

    fn store_envelope(body: Value) -> RequestEnvelope<Value> {
        let mut env = envelope(Some("dimse"), Some(Command::new(Operation::Store)));
        env.request_details
            .metadata
            .insert("SOPInstanceUID".to_string(), "1.2.3.4.5".to_string());
        env.original_data = body.clone();
        env.normalized_data = Some(body);
        env
    }

    #[tokio::test]
    async fn streaming_store_sends_the_instance_and_drops_the_local_path() {
        let env = store_envelope(serde_json::json!({
            "command": { "sop_class_uid": "1.2.840.10008.5.1.4.1.1.2" },
            "dataset": { "00100020": { "vr": "LO", "Value": ["PID1"] } },
            "file": "/var/harmony/dimse/instance.dcm",
            "part10_b64": "UGFydDEw",
            "transfer_syntax_uid": "1.2.840.10008.1.2.1",
        }));

        let out = DicomToHttpMiddleware::new().left(env).await.expect("left");

        // The executor turns a lone `body_b64` into the raw request body.
        assert_eq!(out.original_data, serde_json::json!({ "body_b64": "UGFydDEw" }));
        let header = |name: &str| out.request_details.headers.get(name).map(String::as_str);
        assert_eq!(header("content-type"), Some("application/dicom"));
        assert_eq!(header("x-dimse-op"), Some("store"));
        assert_eq!(header("x-dicom-transfer-syntax"), Some("1.2.840.10008.1.2.1"));
        assert_eq!(header("x-dicom-sop-class-uid"), Some("1.2.840.10008.5.1.4.1.1.2"));
        assert_eq!(header("x-dicom-sop-instance-uid"), Some("1.2.3.4.5"));
    }

    #[tokio::test]
    async fn store_without_an_inline_instance_keeps_its_json_body() {
        // In front of the `http` backend the adapter adds no `part10_b64`.
        // The DICOM JSON body must pass through as it is.
        let body = serde_json::json!({
            "command": { "sop_class_uid": "1.2.840.10008.5.1.4.1.1.2" },
            "dataset": { "00100020": { "vr": "LO", "Value": ["PID1"] } },
            "file": "/var/harmony/dimse/instance.dcm",
        });

        let out = DicomToHttpMiddleware::new()
            .left(store_envelope(body.clone()))
            .await
            .expect("left");

        assert_eq!(out.original_data, body);
        let headers = &out.request_details.headers;
        assert_eq!(headers.get("content-type").map(String::as_str), Some("application/json"));
        assert_eq!(headers.get("x-dimse-op").map(String::as_str), Some("store"));
        assert!(!headers.contains_key("x-dicom-transfer-syntax"));
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
