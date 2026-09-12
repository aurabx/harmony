use crate::config::config::ConfigError;
use crate::models::envelope::command::Command;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::models::services::streaming::{ResponseStream, StreamingBackend, StreamMetadata};
use async_trait::async_trait;
use axum::response::Response;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;

use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use dicom_json_tool as djt;
use dimse::types::{DatasetStream, FindQuery, GetQuery, QueryLevel};
use dimse::{DimseConfig, DimseScu, RemoteNode};
use tracing::{debug, info, warn};

/// DICOM SCU (Service Class User) streaming backend configuration
///
/// This service handles outgoing DICOM DIMSE requests (C-FIND, C-GET) and yields
/// each DICOM instance as a separate ResponseEnvelope in a stream.
///
/// Unlike the standard `dicom_scu` backend which collects all instances before
/// returning, this backend streams instances as they arrive from the PACS, enabling:
/// - Lower memory usage for large studies
/// - Progressive processing through middleware
/// - Early client response (first instance arrives sooner)
/// - Proper backpressure handling
#[derive(Debug, Deserialize)]
pub struct DicomScuStreamingBackend {
    pub local_aet: Option<String>,
    pub aet: Option<String>, // Remote AET
    pub host: Option<String>,
    pub port: Option<u16>,
    pub use_tls: Option<bool>,
}

impl DicomScuStreamingBackend {
    /// Get the local AET from options or struct, with default fallback
    fn get_local_aet(&self, options: &HashMap<String, Value>) -> Option<String> {
        options
            .get("local_aet")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.local_aet.clone())
            .or_else(|| Some("HARMONY_STREAM".to_string()))
    }

    /// Create a remote node from configuration
    fn create_remote_node(
        &self,
        options: &HashMap<String, Value>,
    ) -> Result<RemoteNode, ConfigError> {
        let aet = options
            .get("aet")
            .and_then(|v| v.as_str())
            .or(self.aet.as_deref())
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom_scu_streaming".to_string(),
                reason: "Missing remote 'aet' (Application Entity Title)".to_string(),
            })?
            .to_string();

        let host = options
            .get("host")
            .and_then(|v| v.as_str())
            .or(self.host.as_deref())
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom_scu_streaming".to_string(),
                reason: "Missing 'host' (DICOM server address)".to_string(),
            })?
            .to_string();

        let port = options
            .get("port")
            .and_then(|v| v.as_u64())
            .or(self.port.map(|p| p as u64))
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom_scu_streaming".to_string(),
                reason: "Missing 'port'".to_string(),
            })?;

        if !(1..=65535).contains(&port) {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom_scu_streaming".to_string(),
                reason: "Invalid 'port' (Allowed range: 1-65535)".to_string(),
            });
        }

        let mut node = RemoteNode::new(aet, host, port as u16);

        if options
            .get("use_tls")
            .and_then(|v| v.as_bool())
            .or(self.use_tls)
            .unwrap_or(false)
        {
            node = node.with_tls();
        }

        Ok(node)
    }

    /// Extract query parameters from envelope
    fn extract_query_params(
        &self,
        envelope: &RequestEnvelope<Vec<u8>>,
    ) -> Result<HashMap<String, String>, Error> {
        let body_json: serde_json::Value = serde_json::from_slice(&envelope.original_data)
            .unwrap_or(serde_json::Value::Null);

        // Extract identifier JSON (allow override from normalized_data.dimse_identifier)
        let mut identifier_json = match body_json {
            serde_json::Value::Object(_) => {
                let (_cmd, ident, _qmeta) = djt::parse_wrapper_or_identifier(&body_json);
                ident
            }
            _ => serde_json::json!({}),
        };

        if let Some(nd) = envelope.normalized_data.as_ref() {
            if let Some(ident) = nd.get("dimse_identifier") {
                if ident.is_object() {
                    identifier_json = ident.clone();
                }
            }
        }

        // Flatten identifier JSON into tag->string map
        let mut params: HashMap<String, String> = HashMap::new();
        if let Some(map) = identifier_json.as_object() {
            for (tag, entry) in map.iter() {
                if let Some(val_array) = entry.get("Value").and_then(|v| v.as_array()) {
                    if let Some(first) = val_array.first() {
                        if let Some(s) = first.as_str() {
                            params.insert(tag.clone(), s.to_string());
                        } else if let Some(obj) = first.as_object() {
                            // PN case: { Alphabetic: "..." }
                            if let Some(alpha) = obj.get("Alphabetic").and_then(|v| v.as_str()) {
                                params.insert(tag.clone(), alpha.to_string());
                            }
                        }
                    } else {
                        // Empty array indicates return key
                        params.insert(tag.clone(), String::new());
                    }
                }
            }
        }

        Ok(params)
    }

    /// Build a GetQuery from query parameters
    fn build_get_query(&self, params: &HashMap<String, String>) -> GetQuery {
        let mut get_q = GetQuery::new(QueryLevel::Study);
        for (k, v) in params.iter() {
            get_q = get_q.with_parameter(k.clone(), v.clone());
        }
        get_q
    }

    /// Build a FindQuery from query parameters
    ///
    /// If `explicit_level` is provided, it takes precedence over inference.
    fn build_find_query(&self, params: &HashMap<String, String>, explicit_level: Option<QueryLevel>) -> FindQuery {
        // Use explicit level if provided, otherwise infer from parameters
        let query_level = explicit_level.unwrap_or_else(|| {
            // Determine appropriate query level based on filters and return keys
            if params.get("00080018").is_some_and(|v| !v.is_empty()) {
                QueryLevel::Image
            } else if params.contains_key("00080018")
                && (params.get("0020000D").is_some_and(|v| !v.is_empty())
                    || params.get("0020000E").is_some_and(|v| !v.is_empty()))
            {
                QueryLevel::Image
            } else if params.get("0020000E").is_some_and(|v| !v.is_empty()) {
                QueryLevel::Series
            } else if params.contains_key("0020000E")
                && params.get("0020000D").is_some_and(|v| !v.is_empty())
            {
                QueryLevel::Series
            } else if params.get("0020000D").is_some_and(|v| !v.is_empty()) {
                QueryLevel::Study
            } else if params.contains_key("0020000D") {
                QueryLevel::Study
            } else {
                QueryLevel::Patient
            }
        });

        let mut query = FindQuery::patient(params.get("00100020").cloned());
        query.query_level = query_level;
        for (k, v) in params.iter() {
            query = query.with_parameter(k.clone(), v.clone());
        }
        query
    }

    /// Parse query level from string (e.g. from HTTP header)
    fn parse_query_level(level_str: &str) -> Option<QueryLevel> {
        match level_str.to_uppercase().as_str() {
            "PATIENT" => Some(QueryLevel::Patient),
            "STUDY" => Some(QueryLevel::Study),
            "SERIES" => Some(QueryLevel::Series),
            "IMAGE" | "INSTANCE" => Some(QueryLevel::Image),
            _ => None,
        }
    }
}

#[async_trait]
impl ServiceType for DicomScuStreamingBackend {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate remote connection parameters
        self.create_remote_node(options)?;
        Ok(())
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Backend usage only - no HTTP routes
        vec![]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        if ctx.protocol != crate::models::protocol::Protocol::Dimse {
            return Err(Error::from(
                "DicomScuStreamingBackend only supports Protocol::Dimse in build_protocol_envelope",
            ));
        }

        let metadata: HashMap<String, String> = ctx.meta.clone();
        let op = metadata
            .get("operation")
            .cloned()
            .unwrap_or_else(|| "DIMSE".into());
        let uri = format!("mock-dicom://scp/{}", op.to_lowercase());

        let normalized: Option<serde_json::Value> = serde_json::from_slice(&ctx.payload).ok();

        RequestEnvelope::builder()
            .method(op)
            .uri(uri)
            .headers(HashMap::new())
            .cookies(HashMap::new())
            .query_params(HashMap::new())
            .cache_status(None)
            .metadata(metadata)
            .target_details(None)
            .command(ctx.command)
            .original_data(ctx.payload)
            .normalized_data(normalized)
            .normalized_snapshot(None)
            .build()
    }
}

#[async_trait]
impl ServiceHandler<Value> for DicomScuStreamingBackend {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        _envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "DicomScuStreamingBackend cannot be used as an endpoint (use dicom_scp instead)",
        ))
    }

    async fn backend_outgoing_request(
        &self,
        _envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "DicomScuStreamingBackend requires streaming execution - use execute_streaming()",
        ))
    }

    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));
        envelope
            .response_details
            .metadata
            .insert("service".to_string(), "dicom_streaming".to_string());
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        _envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        Err(Error::from(
            "DicomScuStreamingBackend doesn't support endpoint_outgoing_response",
        ))
    }
}

#[async_trait]
impl StreamingBackend for DicomScuStreamingBackend {
    async fn backend_outgoing_stream(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseStream, Error> {
        info!("Starting DICOM SCU streaming operation");

        // Create remote node and SCU
        let remote_node = self
            .create_remote_node(options)
            .map_err(|e| Error::from(format!("Failed to create remote node: {:?}", e)))?;

        let local_aet = self
            .get_local_aet(options)
            .unwrap_or_else(|| "HARMONY_STREAM".to_string());

        let dimse_config = DimseConfig {
            local_aet: local_aet.clone(),
            ..Default::default()
        };

        let scu = DimseScu::new(dimse_config);

        // Determine operation from:
        // 1. target_details.metadata.dimse_op (set by pipeline routing)
        // 2. request_details.metadata.dimse_op (set by DIMSE SCP)
        // 3. x-dimse-op HTTP header (set by dicom_to_http middleware for cross-instance bridging)
        // 4. Default to "get"
        let op = envelope
            .target_details
            .as_ref()
            .and_then(|td| td.metadata.get("dimse_op"))
            .cloned()
            .or_else(|| envelope.request_details.metadata.get("dimse_op").cloned())
            .or_else(|| envelope.request_details.headers.get("x-dimse-op").cloned())
            .unwrap_or_else(|| "get".to_string());

        let op_normalized = op.trim_start_matches('/').to_lowercase();

        // Extract query parameters
        let query_params = self.extract_query_params(&envelope)?;

        // Extract explicit query level from:
        // 1. x-query-level HTTP header (set by dicom_to_http middleware)
        // 2. request_details.metadata.query_level
        let explicit_query_level = envelope
            .request_details
            .headers
            .get("x-query-level")
            .or_else(|| envelope.request_details.metadata.get("query_level"))
            .and_then(|s| Self::parse_query_level(s));

        debug!("DIMSE streaming operation: {}, params: {:?}, explicit_level: {:?}", 
            op_normalized, query_params, explicit_query_level);

        match op_normalized.as_str() {
            "get" => {
                // Build query and execute C-GET
                let query = self.build_get_query(&query_params);
                let dimse_stream = scu
                    .get_request(&remote_node, query, None)
                    .await
                    .map_err(|e| Error::from(format!("DIMSE C-GET failed: {}", e)))?;

                // Create stream metadata
                let stream_metadata = StreamMetadata::new("dimse-get")
                    .with_metadata("calling_aet", local_aet)
                    .with_metadata("called_aet", remote_node.ae_title.clone());

                // Convert DIMSE stream to ResponseEnvelope stream
                let request_details = envelope.request_details.clone();
                let command = envelope.command.clone();
                let envelope_stream = Self::convert_dimse_stream_to_envelopes(
                    Box::pin(dimse_stream),
                    request_details,
                    stream_metadata,
                    command,
                );

                Ok(Box::pin(envelope_stream))
            }
            "find" => {
                // Build query and execute C-FIND with explicit level if provided
                let query = self.build_find_query(&query_params, explicit_query_level);
                let dimse_stream = scu
                    .find(&remote_node, query)
                    .await
                    .map_err(|e| Error::from(format!("DIMSE C-FIND failed: {}", e)))?;

                let stream_metadata = StreamMetadata::new("dimse-find")
                    .with_metadata("calling_aet", local_aet)
                    .with_metadata("called_aet", remote_node.ae_title.clone());

                let request_details = envelope.request_details.clone();
                let command = envelope.command.clone();
                let envelope_stream = Self::convert_dimse_stream_to_envelopes(
                    Box::pin(dimse_stream),
                    request_details,
                    stream_metadata,
                    command,
                );

                Ok(Box::pin(envelope_stream))
            }
            _ => Err(Error::from(format!(
                "Unsupported streaming operation: '{}'. Supported: get, find",
                op
            ))),
        }
    }
}

impl DicomScuStreamingBackend {
    /// Convert a DIMSE DatasetStream to a stream of ResponseEnvelopes
    fn convert_dimse_stream_to_envelopes(
        dimse_stream: Pin<Box<dyn Stream<Item = Result<DatasetStream, dimse::DimseError>> + Send>>,
        request_details: crate::models::envelope::envelope::RequestDetails,
        _stream_metadata: StreamMetadata,
        command: Option<Command>,
    ) -> impl Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> {
        let mut sequence = 0u64;

        dimse_stream.map(move |result| {
            sequence += 1;

            match result {
                Ok(DatasetStream::File { ref path, ref metadata, .. }) => {
                    debug!("Processing DICOM file: {:?}", path);

                    // Read DICOM file
                    let bytes = std::fs::read(path)
                        .map_err(|e| Error::from(format!("Failed to read DICOM file: {}", e)))?;

                    // Parse DICOM object for metadata
                    let obj = dicom_object::open_file(path)
                        .map_err(|e| Error::from(format!("Failed to parse DICOM file: {}", e)))?;

                    // Convert to JSON for middleware processing
                    let json = dicom_json_tool::identifier_to_json_value(&obj)
                        .map_err(|e| Error::from(format!("Failed to convert DICOM to JSON: {}", e)))?;

                    // Create headers with DICOM metadata
                    let mut headers = HashMap::new();
                    headers.insert("content-type".to_string(), "application/dicom".to_string());

                    if let Some(ref sop_instance_uid) = metadata.sop_instance_uid {
                        headers.insert("x-dicom-sop-instance-uid".to_string(), sop_instance_uid.clone());
                    }
                    if let Some(ref sop_class_uid) = metadata.sop_class_uid {
                        headers.insert("x-dicom-sop-class-uid".to_string(), sop_class_uid.clone());
                    }

                    // Create envelope metadata
                    let mut envelope_metadata = HashMap::new();
                    envelope_metadata.insert("stream_sequence".to_string(), sequence.to_string());
                    envelope_metadata.insert("instance_metadata".to_string(), "true".to_string());

                    if let Some(ref sop_instance_uid) = metadata.sop_instance_uid {
                        envelope_metadata.insert("sop_instance_uid".to_string(), sop_instance_uid.clone());
                    }

                    // Create ResponseEnvelope for this instance
                    let mut response = ResponseEnvelope::from_backend_with_command(
                        request_details.clone(),
                        200,
                        headers,
                        bytes,
                        Some(envelope_metadata),
                        command.clone(),
                    );

                    // Set normalized_data for middleware
                    response.normalized_data = Some(json);

                    // Cleanup temp file
                    let _ = std::fs::remove_file(path);

                    Ok(response)
                }
                Ok(DatasetStream::Memory { ref data, ref metadata }) => {
                    debug!("Processing in-memory DICOM dataset");

                    // Parse DICOM object
                    let obj = dicom_object::from_reader(&**data)
                        .map_err(|e| Error::from(format!("Failed to parse DICOM: {}", e)))?;

                    let json = dicom_json_tool::identifier_to_json_value(&obj)
                        .map_err(|e| Error::from(format!("Failed to convert DICOM to JSON: {}", e)))?;

                    let mut headers = HashMap::new();
                    headers.insert("content-type".to_string(), "application/json".to_string());

                    let mut envelope_metadata = HashMap::new();
                    envelope_metadata.insert("stream_sequence".to_string(), sequence.to_string());

                    if let Some(ref sop_instance_uid) = metadata.sop_instance_uid {
                        envelope_metadata.insert("sop_instance_uid".to_string(), sop_instance_uid.clone());
                    }

                    let mut response = ResponseEnvelope::from_backend_with_command(
                        request_details.clone(),
                        200,
                        headers,
                        serde_json::to_vec(&json).unwrap_or_default(),
                        Some(envelope_metadata),
                        command.clone(),
                    );

                    response.normalized_data = Some(json);

                    Ok(response)
                }
                Ok(DatasetStream::Object { ref object, .. }) => {
                    debug!("Processing DICOM object directly");

                    let json = dicom_json_tool::identifier_to_json_value(object)
                        .map_err(|e| Error::from(format!("Failed to convert DICOM to JSON: {}", e)))?;

                    let mut headers = HashMap::new();
                    headers.insert("content-type".to_string(), "application/json".to_string());

                    let mut response = ResponseEnvelope::from_backend_with_command(
                        request_details.clone(),
                        200,
                        headers,
                        serde_json::to_vec(&json).unwrap_or_default(),
                        None,
                        command.clone(),
                    );

                    response.normalized_data = Some(json);

                    Ok(response)
                }
                Err(e) => {
                    warn!("DIMSE stream error: {}", e);
                    Err(Error::from(format!("DIMSE stream error: {}", e)))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::command::{Command, Operation};
    use crate::models::envelope::envelope::RequestDetails;
    use dimse::types::{DatasetMetadata, DatasetStream, QueryLevel};
    use futures_util::StreamExt;
    use std::collections::HashMap;

    // ── helpers ─────────────────────────────────────────────────────────────

    fn make_backend(aet: Option<&str>, host: Option<&str>, port: Option<u16>) -> DicomScuStreamingBackend {
        DicomScuStreamingBackend {
            local_aet: None,
            aet: aet.map(|s| s.to_string()),
            host: host.map(|s| s.to_string()),
            port,
            use_tls: None,
        }
    }

    fn make_options(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn dummy_request_details() -> RequestDetails {
        RequestDetails {
            method: "GET".to_string(),
            uri: "dimse://scu/get".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata: HashMap::new(),
            content_metadata: None,
        }
    }

    // ── parse_query_level ────────────────────────────────────────────────────

    #[test]
    fn test_parse_query_level_patient() {
        assert_eq!(DicomScuStreamingBackend::parse_query_level("PATIENT"), Some(QueryLevel::Patient));
        assert_eq!(DicomScuStreamingBackend::parse_query_level("patient"), Some(QueryLevel::Patient));
    }

    #[test]
    fn test_parse_query_level_study() {
        assert_eq!(DicomScuStreamingBackend::parse_query_level("STUDY"), Some(QueryLevel::Study));
        assert_eq!(DicomScuStreamingBackend::parse_query_level("study"), Some(QueryLevel::Study));
    }

    #[test]
    fn test_parse_query_level_series() {
        assert_eq!(DicomScuStreamingBackend::parse_query_level("SERIES"), Some(QueryLevel::Series));
    }

    #[test]
    fn test_parse_query_level_image() {
        assert_eq!(DicomScuStreamingBackend::parse_query_level("IMAGE"), Some(QueryLevel::Image));
        assert_eq!(DicomScuStreamingBackend::parse_query_level("INSTANCE"), Some(QueryLevel::Image));
        assert_eq!(DicomScuStreamingBackend::parse_query_level("instance"), Some(QueryLevel::Image));
    }

    #[test]
    fn test_parse_query_level_unknown_returns_none() {
        assert_eq!(DicomScuStreamingBackend::parse_query_level("FRAME"), None);
        assert_eq!(DicomScuStreamingBackend::parse_query_level(""), None);
        assert_eq!(DicomScuStreamingBackend::parse_query_level("GARBAGE"), None);
    }

    // ── build_find_query level inference ────────────────────────────────────

    #[test]
    fn test_build_find_query_explicit_level_takes_precedence() {
        let backend = make_backend(None, None, None);
        let params = HashMap::new();
        // Explicit Study overrides default Patient
        let query = backend.build_find_query(&params, Some(QueryLevel::Study));
        assert_eq!(query.query_level, QueryLevel::Study);
    }

    #[test]
    fn test_build_find_query_infers_patient_from_empty_params() {
        let backend = make_backend(None, None, None);
        let params = HashMap::new();
        let query = backend.build_find_query(&params, None);
        assert_eq!(query.query_level, QueryLevel::Patient);
    }

    #[test]
    fn test_build_find_query_infers_study_from_study_uid() {
        let backend = make_backend(None, None, None);
        let mut params = HashMap::new();
        // 0020000D = StudyInstanceUID (non-empty → Study level)
        params.insert("0020000D".to_string(), "1.2.3".to_string());
        let query = backend.build_find_query(&params, None);
        assert_eq!(query.query_level, QueryLevel::Study);
    }

    #[test]
    fn test_build_find_query_infers_series_from_series_uid() {
        let backend = make_backend(None, None, None);
        let mut params = HashMap::new();
        // 0020000E = SeriesInstanceUID (non-empty → Series level)
        params.insert("0020000E".to_string(), "1.2.3.4".to_string());
        let query = backend.build_find_query(&params, None);
        assert_eq!(query.query_level, QueryLevel::Series);
    }

    #[test]
    fn test_build_find_query_infers_image_from_sop_instance_uid() {
        let backend = make_backend(None, None, None);
        let mut params = HashMap::new();
        // 00080018 = SOPInstanceUID (non-empty → Image level)
        params.insert("00080018".to_string(), "1.2.3.4.5".to_string());
        let query = backend.build_find_query(&params, None);
        assert_eq!(query.query_level, QueryLevel::Image);
    }

    #[test]
    fn test_build_find_query_copies_all_params() {
        let backend = make_backend(None, None, None);
        let mut params = HashMap::new();
        params.insert("00100020".to_string(), "PAT001".to_string());
        params.insert("00100010".to_string(), "Doe^John".to_string());
        let query = backend.build_find_query(&params, None);
        assert!(query.parameters.contains_key("00100020"));
        assert!(query.parameters.contains_key("00100010"));
    }

    // ── build_get_query ──────────────────────────────────────────────────────

    #[test]
    fn test_build_get_query_defaults_to_study_level() {
        let backend = make_backend(None, None, None);
        let params = HashMap::new();
        let query = backend.build_get_query(&params);
        assert_eq!(query.query_level, QueryLevel::Study);
    }

    #[test]
    fn test_build_get_query_copies_params() {
        let backend = make_backend(None, None, None);
        let mut params = HashMap::new();
        params.insert("0020000D".to_string(), "1.2.3".to_string());
        let query = backend.build_get_query(&params);
        assert_eq!(query.parameters.get("0020000D").map(|s| s.as_str()), Some("1.2.3"));
    }

    // ── get_local_aet ────────────────────────────────────────────────────────

    #[test]
    fn test_get_local_aet_from_options() {
        let backend = make_backend(None, None, None);
        let options = make_options(&[("local_aet", serde_json::json!("FROM_OPTIONS"))]);
        assert_eq!(backend.get_local_aet(&options), Some("FROM_OPTIONS".to_string()));
    }

    #[test]
    fn test_get_local_aet_from_struct_when_no_option() {
        let mut backend = make_backend(None, None, None);
        backend.local_aet = Some("FROM_STRUCT".to_string());
        let options = make_options(&[]);
        assert_eq!(backend.get_local_aet(&options), Some("FROM_STRUCT".to_string()));
    }

    #[test]
    fn test_get_local_aet_default_fallback() {
        let backend = make_backend(None, None, None);
        let options = make_options(&[]);
        assert_eq!(backend.get_local_aet(&options), Some("HARMONY_STREAM".to_string()));
    }

    #[test]
    fn test_get_local_aet_options_take_precedence_over_struct() {
        let mut backend = make_backend(None, None, None);
        backend.local_aet = Some("FROM_STRUCT".to_string());
        let options = make_options(&[("local_aet", serde_json::json!("FROM_OPTIONS"))]);
        assert_eq!(backend.get_local_aet(&options), Some("FROM_OPTIONS".to_string()));
    }

    // ── create_remote_node ───────────────────────────────────────────────────

    #[test]
    fn test_create_remote_node_success() {
        let backend = make_backend(None, None, None);
        let options = make_options(&[
            ("aet", serde_json::json!("REMOTE_AET")),
            ("host", serde_json::json!("pacs.example.com")),
            ("port", serde_json::json!(4242)),
        ]);
        assert!(backend.create_remote_node(&options).is_ok());
    }

    #[test]
    fn test_create_remote_node_missing_aet_fails() {
        let backend = make_backend(None, None, None);
        let options = make_options(&[
            ("host", serde_json::json!("pacs.example.com")),
            ("port", serde_json::json!(4242)),
        ]);
        let err = backend.create_remote_node(&options).unwrap_err();
        assert!(format!("{:?}", err).contains("aet"));
    }

    #[test]
    fn test_create_remote_node_missing_host_fails() {
        let backend = make_backend(None, None, None);
        let options = make_options(&[
            ("aet", serde_json::json!("REMOTE_AET")),
            ("port", serde_json::json!(4242)),
        ]);
        let err = backend.create_remote_node(&options).unwrap_err();
        assert!(format!("{:?}", err).contains("host"));
    }

    #[test]
    fn test_create_remote_node_missing_port_fails() {
        let backend = make_backend(None, None, None);
        let options = make_options(&[
            ("aet", serde_json::json!("REMOTE_AET")),
            ("host", serde_json::json!("pacs.example.com")),
        ]);
        assert!(backend.create_remote_node(&options).is_err());
    }

    #[test]
    fn test_create_remote_node_invalid_port_zero_fails() {
        let backend = make_backend(None, None, None);
        let options = make_options(&[
            ("aet", serde_json::json!("REMOTE_AET")),
            ("host", serde_json::json!("pacs.example.com")),
            ("port", serde_json::json!(0u64)),
        ]);
        assert!(backend.create_remote_node(&options).is_err());
    }

    #[test]
    fn test_create_remote_node_uses_struct_fields_as_fallback() {
        let backend = make_backend(Some("STRUCT_AET"), Some("struct.host"), Some(1234));
        let options = make_options(&[]);
        assert!(backend.create_remote_node(&options).is_ok());
    }

    // ── convert_dimse_stream_to_envelopes: Command propagation ───────────────

    #[tokio::test]
    async fn test_convert_stream_propagates_command_to_envelopes() {
        use dicom_object::InMemDicomObject;
        use futures_util::stream;

        let command = Command::new(Operation::Retrieve).with_source("test");
        let request_details = dummy_request_details();

        // Build a minimal in-memory DICOM object (empty but valid)
        let obj = InMemDicomObject::new_empty();
        let mut metadata = DatasetMetadata::new();
        metadata.sop_instance_uid = Some("1.2.3.4.5".to_string());

        let dataset = DatasetStream::Object { object: obj, metadata };
        let dimse_stream: Pin<Box<dyn Stream<Item = Result<DatasetStream, dimse::DimseError>> + Send>> =
            Box::pin(stream::once(async { Ok(dataset) }));

        let stream_metadata = StreamMetadata::new("dimse-get");

        let mut result_stream = DicomScuStreamingBackend::convert_dimse_stream_to_envelopes(
            dimse_stream,
            request_details,
            stream_metadata,
            Some(command.clone()),
        );

        let item = result_stream.next().await.expect("stream should yield one item");
        let envelope = item.expect("item should be Ok");

        // Command is propagated to the response envelope
        let cmd = envelope.command.expect("command should be Some");
        assert_eq!(cmd.operation, Operation::Retrieve);
        assert_eq!(cmd.source.as_deref(), Some("test"));
    }

    #[tokio::test]
    async fn test_convert_stream_none_command_propagated() {
        use dicom_object::InMemDicomObject;
        use futures_util::stream;

        let request_details = dummy_request_details();
        let obj = InMemDicomObject::new_empty();
        let metadata = DatasetMetadata::new();
        let dataset = DatasetStream::Object { object: obj, metadata };

        let dimse_stream: Pin<Box<dyn Stream<Item = Result<DatasetStream, dimse::DimseError>> + Send>> =
            Box::pin(stream::once(async { Ok(dataset) }));

        let stream_metadata = StreamMetadata::new("dimse-get");

        let mut result_stream = DicomScuStreamingBackend::convert_dimse_stream_to_envelopes(
            dimse_stream,
            request_details,
            stream_metadata,
            None,
        );

        let item = result_stream.next().await.expect("stream should yield one item");
        let envelope = item.expect("item should be Ok");

        assert!(envelope.command.is_none(), "None command should propagate as None");
    }

    #[tokio::test]
    async fn test_convert_stream_dimse_error_propagates() {
        use futures_util::stream;

        let request_details = dummy_request_details();
        let err = dimse::DimseError::operation_failed("test error");
        let dimse_stream: Pin<Box<dyn Stream<Item = Result<DatasetStream, dimse::DimseError>> + Send>> =
            Box::pin(stream::once(async { Err(err) }));

        let stream_metadata = StreamMetadata::new("dimse-get");
        let mut result_stream = DicomScuStreamingBackend::convert_dimse_stream_to_envelopes(
            dimse_stream,
            request_details,
            stream_metadata,
            None,
        );

        let item = result_stream.next().await.expect("stream should yield one item");
        assert!(item.is_err(), "DIMSE error should surface as Err");
    }

    // ── run_streaming op-string derivation ───────────────────────────────────
    //
    // The full `run_streaming()` requires global config and live services, so we
    // test the op-derivation mapping directly — the same match arm used there.

    fn derive_op(operation: Operation) -> &'static str {
        match operation {
            Operation::Query    => "find",
            Operation::Retrieve => "get",
            Operation::Move     => "move",
            Operation::Store    => "store",
            Operation::Ping     => "echo",
            _                   => "unknown",
        }
    }

    #[test]
    fn test_op_derivation_query_maps_to_find() {
        assert_eq!(derive_op(Operation::Query), "find");
    }

    #[test]
    fn test_op_derivation_retrieve_maps_to_get() {
        assert_eq!(derive_op(Operation::Retrieve), "get");
    }

    #[test]
    fn test_op_derivation_move_maps_to_move() {
        assert_eq!(derive_op(Operation::Move), "move");
    }

    #[test]
    fn test_op_derivation_store_maps_to_store() {
        assert_eq!(derive_op(Operation::Store), "store");
    }

    #[test]
    fn test_op_derivation_ping_maps_to_echo() {
        assert_eq!(derive_op(Operation::Ping), "echo");
    }

    #[test]
    fn test_op_derivation_custom_maps_to_unknown() {
        assert_eq!(derive_op(Operation::Custom), "unknown");
    }
}
