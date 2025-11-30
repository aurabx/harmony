use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, warn};

/// DICOM status codes for response mapping
#[allow(dead_code)]
mod dicom_status {
    pub const SUCCESS: u16 = 0x0000;
    pub const PENDING: u16 = 0xFF00;
    pub const CANCEL: u16 = 0xFE00;
    pub const FAILURE_UNABLE_TO_PROCESS: u16 = 0xC000;
    pub const FAILURE_OUT_OF_RESOURCES: u16 = 0xA700;
    pub const FAILURE_IDENTIFIER_DOES_NOT_MATCH: u16 = 0xA900;
    pub const WARNING_SUBOPS_COMPLETE_WITH_FAILURES: u16 = 0xB000;
}

/// Middleware that bridges DICOM DIMSE requests to DICOMweb HTTP requests.
///
/// LEFT side: Converts DIMSE operations (C-FIND, C-STORE, C-GET) to DICOMweb requests (QIDO-RS, STOW-RS, WADO-RS)
/// RIGHT side: Converts DICOMweb responses to DIMSE-compatible JSON/Status
#[derive(Default, Debug)]
pub struct DicomToDicomwebMiddleware;

impl DicomToDicomwebMiddleware {
    pub fn new() -> Self {
        Self
    }

    /// Build multipart body for STOW-RS
    fn build_stow_multipart(file_path: &str) -> Result<(String, Vec<u8>), Error> {
        let path = PathBuf::from(file_path);
        let bytes = std::fs::read(&path).map_err(|e| Error::from(format!("Failed to read DICOM file: {}", e)))?;

        let boundary = format!("boundary_{}", uuid::Uuid::new_v4());
        let mut body = Vec::new();

        // Preamble
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Type: application/dicom\r\n\r\n");
        body.extend_from_slice(&bytes);
        body.extend_from_slice(b"\r\n");
        
        // Terminator
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        Ok((boundary, body))
    }
}

#[async_trait]
impl Middleware for DicomToDicomwebMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<Value>, Error> {
        // Only process if we have a dimse operation metadata
        let op = match envelope.request_details.metadata.get("operation") {
            Some(o) => o.to_uppercase(),
            None => return Ok(envelope),
        };

        // If protocol is not dimse (e.g. already converted or HTTP source), skip
        // But here we expect to be transforming the request to HTTP for the backend
        // so we change the protocol context effectively.

        match op.as_str() {
            "C-FIND" => {
                self.transform_cfind_request(&mut envelope)?;
            }
            "C-STORE" => {
                self.transform_cstore_request(&mut envelope)?;
            }
            "C-GET" | "C-MOVE" => {
                self.transform_cget_cmove_request(&mut envelope)?;
            }
            _ => {}
        }

        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: ResponseEnvelope<Value>,
    ) -> Result<ResponseEnvelope<Value>, Error> {
        let op = envelope
            .request_details
            .metadata
            .get("operation")
            .map(|s| s.as_str())
            .unwrap_or("");

        let http_status = envelope.response_details.status;
        debug!(
            "dicom_to_dicomweb right: operation={}, http_status={}",
            op, http_status
        );

        // Convert HTTP response to expected DIMSE result
        match op {
            "C-FIND" => {
                self.transform_cfind_response(&mut envelope)?;
            }
            "C-STORE" => {
                self.transform_cstore_response(&mut envelope)?;
            }
            "C-GET" | "C-MOVE" => {
                self.transform_cget_cmove_response(&mut envelope)?;
            }
            _ => {
                debug!("Unknown or missing operation in response: {}", op);
            }
        }

        Ok(envelope)
    }
}

impl DicomToDicomwebMiddleware {
    /// Transform C-FIND request from DIMSE to QIDO-RS GET
    fn transform_cfind_request(
        &self,
        envelope: &mut RequestEnvelope<Value>,
    ) -> Result<(), Error> {
        let nd = envelope.normalized_data.clone().unwrap_or(Value::Null);
        let wrapper = nd;
        let identifier = wrapper.get("identifier").and_then(|v| v.as_object());

        if let Some(ident) = identifier {
            let mut query_params = HashMap::new();

            for (tag, value) in ident {
                // Extract value from DICOM JSON model {"vr": "...", "Value": [...]}
                if let Some(vals) = value.get("Value").and_then(|v| v.as_array()) {
                    if !vals.is_empty() {
                        // Use the first value for query
                        // TODO: Handle range matching, wildcards properly if needed
                        // QIDO supports 00100010=Smith^John
                        let v_str = match &vals[0] {
                            Value::String(s) => s.clone(),
                            Value::Object(o) => {
                                // Handle PN: {"Alphabetic": "Name"}
                                if let Some(alpha) = o.get("Alphabetic").and_then(|s| s.as_str()) {
                                    alpha.to_string()
                                } else {
                                    vals[0].to_string()
                                }
                            }
                            _ => vals[0].to_string(),
                        };

                        query_params.insert(tag.clone(), vec![v_str]);
                    }
                }
            }

            // Update envelope for HTTP backend
            envelope.request_details.method = "GET".to_string();
            envelope.request_details.uri = "/studies".to_string();
            envelope.request_details.query_params = query_params;

            // Clear body as GET doesn't have one
            envelope.original_data = Value::Null;
            envelope.normalized_data = None;
        }

        Ok(())
    }

    /// Transform C-STORE request from DIMSE to STOW-RS POST
    fn transform_cstore_request(
        &self,
        envelope: &mut RequestEnvelope<Value>,
    ) -> Result<(), Error> {
        let nd = envelope.normalized_data.clone().unwrap_or(Value::Null);

        if let Some(file_path) = nd.get("file").and_then(|s| s.as_str()) {
            let (boundary, body) = Self::build_stow_multipart(file_path)?;

            envelope.request_details.method = "POST".to_string();
            envelope.request_details.uri = "/studies".to_string();

            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                format!(
                    "multipart/related; type=\"application/dicom\"; boundary={}",
                    boundary
                ),
            );
            envelope.request_details.headers = headers;

            // Encode body as Base64 in normalized_data for HttpEndpoint to pick up
            let b64 = base64::engine::general_purpose::STANDARD.encode(&body);

            // Update normalized_data
            let mut new_nd = nd.clone();
            if let Some(obj) = new_nd.as_object_mut() {
                obj.insert("body_b64".to_string(), Value::String(b64));
            } else {
                new_nd = serde_json::json!({
                    "body_b64": b64
                });
            }
            envelope.normalized_data = Some(new_nd);
        }

        Ok(())
    }

    /// Transform C-GET/C-MOVE request from DIMSE to WADO-RS GET
    fn transform_cget_cmove_request(
        &self,
        envelope: &mut RequestEnvelope<Value>,
    ) -> Result<(), Error> {
        let nd = envelope.normalized_data.clone().unwrap_or(Value::Null);
        let wrapper = nd;
        let identifier = wrapper.get("identifier").and_then(|v| v.as_object());

        if let Some(ident) = identifier {
            // Extract UIDs
            let study_uid = ident
                .get("0020000D")
                .and_then(|v| v.get("Value"))
                .and_then(|a| a.get(0))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let series_uid = ident
                .get("0020000E")
                .and_then(|v| v.get("Value"))
                .and_then(|a| a.get(0))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let instance_uid = ident
                .get("00080018")
                .and_then(|v| v.get("Value"))
                .and_then(|a| a.get(0))
                .and_then(|s| s.as_str())
                .unwrap_or("");

            let uri = if !instance_uid.is_empty() && !series_uid.is_empty() && !study_uid.is_empty() {
                format!(
                    "/studies/{}/series/{}/instances/{}",
                    study_uid, series_uid, instance_uid
                )
            } else if !series_uid.is_empty() && !study_uid.is_empty() {
                format!("/studies/{}/series/{}", study_uid, series_uid)
            } else if !study_uid.is_empty() {
                format!("/studies/{}", study_uid)
            } else {
                "/studies".to_string()
            };

            envelope.request_details.method = "GET".to_string();
            envelope.request_details.uri = uri;

            let mut headers = HashMap::new();
            // Request multipart/related for instances
            headers.insert(
                "accept".to_string(),
                "multipart/related; type=\"application/dicom\"".to_string(),
            );
            envelope.request_details.headers = headers;

            envelope.original_data = Value::Null;
            envelope.normalized_data = None;
        }

        Ok(())
    }

    /// Transform C-FIND response from QIDO-RS JSON to DIMSE format
    fn transform_cfind_response(
        &self,
        envelope: &mut ResponseEnvelope<Value>,
    ) -> Result<(), Error> {
        let http_status = envelope.response_details.status;

        // Check for HTTP errors
        if http_status >= 400 {
            let dicom_status = Self::http_to_dicom_status(http_status);
            warn!(
                "C-FIND backend returned HTTP {}, mapping to DICOM status 0x{:04X}",
                http_status, dicom_status
            );
            envelope.response_details.metadata.insert(
                "dicom_status".to_string(),
                format!("0x{:04X}", dicom_status),
            );
            return Ok(());
        }

        // QIDO-RS returns JSON array in normalized_data (auto-parsed by HTTP backend)
        // or in original_data if it's already a Value
        let results = envelope
            .normalized_data
            .as_ref()
            .or(Some(&envelope.original_data));

        if let Some(data) = results {
            // Validate it's an array
            if data.is_array() {
                let count = data.as_array().map(|a| a.len()).unwrap_or(0);
                debug!("C-FIND QIDO-RS response contains {} results", count);

                // Store result count in metadata for PipelineQueryProvider
                envelope
                    .response_details
                    .metadata
                    .insert("result_count".to_string(), count.to_string());

                // Mark as successful DICOM status
                envelope.response_details.metadata.insert(
                    "dicom_status".to_string(),
                    format!("0x{:04X}", dicom_status::SUCCESS),
                );
            } else if data.is_null() {
                // Empty response = no matches
                debug!("C-FIND QIDO-RS response is null, treating as empty result set");
                envelope
                    .response_details
                    .metadata
                    .insert("result_count".to_string(), "0".to_string());
                envelope.response_details.metadata.insert(
                    "dicom_status".to_string(),
                    format!("0x{:04X}", dicom_status::SUCCESS),
                );
                // Set normalized_data to empty array
                envelope.normalized_data = Some(serde_json::json!([]));
            } else {
                // Single object response (some servers return this for single match)
                debug!("C-FIND QIDO-RS response is single object, wrapping in array");
                envelope
                    .response_details
                    .metadata
                    .insert("result_count".to_string(), "1".to_string());
                envelope.response_details.metadata.insert(
                    "dicom_status".to_string(),
                    format!("0x{:04X}", dicom_status::SUCCESS),
                );
                // Wrap single object in array for consistent handling
                envelope.normalized_data = Some(serde_json::json!([data.clone()]));
            }
        } else {
            // No data at all
            debug!("C-FIND response has no data");
            envelope
                .response_details
                .metadata
                .insert("result_count".to_string(), "0".to_string());
            envelope.normalized_data = Some(serde_json::json!([]));
        }

        Ok(())
    }

    /// Transform C-STORE response from STOW-RS to DIMSE format
    fn transform_cstore_response(
        &self,
        envelope: &mut ResponseEnvelope<Value>,
    ) -> Result<(), Error> {
        let http_status = envelope.response_details.status;

        // Map HTTP status to DICOM status
        let dicom_status = Self::http_to_dicom_status(http_status);

        if http_status >= 200 && http_status < 300 {
            debug!(
                "C-STORE STOW-RS succeeded with HTTP {}, DICOM status 0x{:04X}",
                http_status, dicom_status
            );

            // Parse STOW-RS response for stored instance UIDs
            if let Some(ref data) = envelope.normalized_data {
                // STOW-RS returns stored instances info
                if data.get("00081199").is_some() {
                    // ReferencedSOPSequence is present
                    debug!("STOW-RS response contains ReferencedSOPSequence");
                }
                if let Some(failed) = data.get("00081198") {
                    // FailedSOPSequence
                    if let Some(arr) = failed.get("Value").and_then(|v| v.as_array()) {
                        if !arr.is_empty() {
                            warn!("STOW-RS reported {} failed instances", arr.len());
                            // Still return success if some instances were stored
                        }
                    }
                }
            }
        } else {
            warn!(
                "C-STORE STOW-RS failed with HTTP {}, DICOM status 0x{:04X}",
                http_status, dicom_status
            );
        }

        // Store DICOM status in metadata
        envelope.response_details.metadata.insert(
            "dicom_status".to_string(),
            format!("0x{:04X}", dicom_status),
        );

        Ok(())
    }

    /// Transform C-GET/C-MOVE response from WADO-RS to DIMSE format
    fn transform_cget_cmove_response(
        &self,
        envelope: &mut ResponseEnvelope<Value>,
    ) -> Result<(), Error> {
        let http_status = envelope.response_details.status;

        // Check for HTTP errors
        if http_status >= 400 {
            let dicom_status = Self::http_to_dicom_status(http_status);
            warn!(
                "C-GET/C-MOVE backend returned HTTP {}, mapping to DICOM status 0x{:04X}",
                http_status, dicom_status
            );
            envelope.response_details.metadata.insert(
                "dicom_status".to_string(),
                format!("0x{:04X}", dicom_status),
            );
            return Ok(());
        }

        // Check content-type for multipart handling
        let content_type = envelope
            .response_details
            .headers
            .get("content-type")
            .or_else(|| envelope.response_details.headers.get("Content-Type"))
            .cloned()
            .unwrap_or_default();

        debug!("C-GET/C-MOVE response content-type: {}", content_type);

        if content_type.contains("multipart/related") {
            // Multipart WADO-RS response containing DICOM instances
            // Parse and extract DICOM parts
            self.parse_multipart_wado_response(envelope, &content_type)?;
        } else if content_type.contains("application/dicom") {
            // Single DICOM instance response
            debug!("C-GET/C-MOVE received single DICOM instance");
            envelope
                .response_details
                .metadata
                .insert("dataset_count".to_string(), "1".to_string());
            envelope.response_details.metadata.insert(
                "dicom_status".to_string(),
                format!("0x{:04X}", dicom_status::SUCCESS),
            );
        } else if content_type.contains("application/dicom+json") {
            // JSON metadata response (WADO-RS metadata endpoint)
            debug!("C-GET/C-MOVE received DICOM JSON metadata");
            envelope.response_details.metadata.insert(
                "dicom_status".to_string(),
                format!("0x{:04X}", dicom_status::SUCCESS),
            );
        } else {
            warn!(
                "C-GET/C-MOVE received unexpected content-type: {}",
                content_type
            );
            envelope.response_details.metadata.insert(
                "dicom_status".to_string(),
                format!("0x{:04X}", dicom_status::FAILURE_UNABLE_TO_PROCESS),
            );
        }

        Ok(())
    }

    /// Parse multipart/related WADO-RS response and extract DICOM instances
    fn parse_multipart_wado_response(
        &self,
        envelope: &mut ResponseEnvelope<Value>,
        content_type: &str,
    ) -> Result<(), Error> {
        // Extract boundary from content-type
        let boundary = content_type
            .split(';')
            .find_map(|part| {
                let part = part.trim();
                if part.starts_with("boundary=") {
                    Some(part.trim_start_matches("boundary=").trim_matches('"'))
                } else {
                    None
                }
            })
            .unwrap_or("");

        if boundary.is_empty() {
            warn!("Multipart response missing boundary");
            envelope.response_details.metadata.insert(
                "dicom_status".to_string(),
                format!("0x{:04X}", dicom_status::FAILURE_UNABLE_TO_PROCESS),
            );
            return Ok(());
        }

        debug!("Parsing multipart WADO-RS response with boundary: {}", boundary);

        // The raw bytes should be in original_data or we need to reconstruct from normalized_data
        // For now, mark as needing multipart parsing and let PipelineQueryProvider handle it
        // In a full implementation, we would:
        // 1. Get raw bytes from the HTTP response
        // 2. Split by boundary
        // 3. Extract each DICOM part
        // 4. Store as array of base64-encoded datasets or file references

        envelope
            .response_details
            .metadata
            .insert("multipart_boundary".to_string(), boundary.to_string());
        envelope
            .response_details
            .metadata
            .insert("response_format".to_string(), "multipart/dicom".to_string());

        // For now, mark as successful and let PipelineQueryProvider handle parsing
        // The raw bytes will be in original_data for it to process
        envelope.response_details.metadata.insert(
            "dicom_status".to_string(),
            format!("0x{:04X}", dicom_status::SUCCESS),
        );

        Ok(())
    }

    /// Map HTTP status code to DICOM status code
    fn http_to_dicom_status(http_status: u16) -> u16 {
        match http_status {
            200..=299 => dicom_status::SUCCESS,
            400 => dicom_status::FAILURE_IDENTIFIER_DOES_NOT_MATCH, // Bad Request
            404 => dicom_status::SUCCESS, // Not Found = no matches (for query)
            409 => dicom_status::WARNING_SUBOPS_COMPLETE_WITH_FAILURES, // Conflict
            413 => dicom_status::FAILURE_OUT_OF_RESOURCES, // Payload Too Large
            500..=599 => dicom_status::FAILURE_UNABLE_TO_PROCESS,
            _ => dicom_status::FAILURE_UNABLE_TO_PROCESS,
        }
    }
}
