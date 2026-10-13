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

use super::dicom_store_body::part10_from_store_body;
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use dicom_json_tool as djt;
use dimse::types::{DatasetStream, FindQuery, GetQuery, QueryLevel};
use dimse::{DimseConfig, DimseScu, RemoteNode};
use futures_util::stream;
use tracing::{debug, info, warn};

/// Request headers in which `dicom_to_http` describes a bridged C-STORE
/// instance. Each one that is present must agree with the instance.
const TRANSFER_SYNTAX_HEADER: &str = "x-dicom-transfer-syntax";
const SOP_CLASS_UID_HEADER: &str = "x-dicom-sop-class-uid";
const SOP_INSTANCE_UID_HEADER: &str = "x-dicom-sop-instance-uid";

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

    /// The `transfer_syntaxes` option: the transfer syntaxes proposed on the
    /// storage presentation contexts of a C-GET, most preferred first. Retrieved
    /// instances arrive in one of them. Defaults to the uncompressed syntaxes.
    fn storage_transfer_syntaxes(
        options: &HashMap<String, Value>,
    ) -> Result<Vec<String>, ConfigError> {
        let invalid = |reason: String| ConfigError::InvalidEndpoint {
            name: "dicom_scu_streaming".to_string(),
            reason,
        };

        let Some(value) = options.get("transfer_syntaxes") else {
            return Ok(DimseConfig::default().storage_transfer_syntaxes);
        };
        let items = value.as_array().ok_or_else(|| {
            invalid("'transfer_syntaxes' must be an array of transfer syntax UIDs".to_string())
        })?;

        let mut list = Vec::with_capacity(items.len());
        for item in items {
            let uid = item.as_str().ok_or_else(|| {
                invalid(format!(
                    "'transfer_syntaxes' entries must be strings, found {}",
                    item
                ))
            })?;
            list.push(uid.trim().to_string());
        }
        dimse::config::validate_storage_transfer_syntaxes(&list)
            .map_err(|e| invalid(format!("'transfer_syntaxes': {}", e)))?;
        Ok(list)
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

    /// Extract the instance for a C-STORE from a request envelope.
    ///
    /// The request must carry the DICOM Part 10 file itself, either as the
    /// whole body or in the `part10_b64` key of a JSON body. The first is what
    /// `dicom_to_http` sends from another Harmony. The second is what the
    /// DIMSE adapter builds when this backend is in the same pipeline.
    ///
    /// A path in the request is never read. A request can come from any HTTP
    /// client, and a path it names would be opened with this process's rights.
    ///
    /// The returned dataset has its SOP UIDs and its transfer syntax, taken
    /// from the file meta group, in its metadata.
    fn extract_store_dataset(envelope: &RequestEnvelope<Vec<u8>>) -> Result<DatasetStream, Error> {
        let part10 = part10_from_store_body(&envelope.original_data)
            .map_err(|e| Error::from(format!("dicom_scu_streaming store: {}", e)))?;

        let file = dicom_object::from_reader(std::io::Cursor::new(part10.as_ref())).map_err(|e| {
            Error::from(format!(
                "dicom_scu_streaming store: the instance is not a readable DICOM Part 10 file: {}",
                e
            ))
        })?;
        let transfer_syntax = file.meta().transfer_syntax().to_string();

        let mut dataset = DatasetStream::from_object(file.into_inner());
        let metadata = dataset.metadata_mut();
        metadata.transfer_syntax = Some(transfer_syntax);
        metadata.size_bytes = Some(part10.len() as u64);

        // Middleware and logs on this host may read these headers, so a
        // header that contradicts the instance must not get through.
        let headers = &envelope.request_details.headers;
        for (header, actual) in [
            (TRANSFER_SYNTAX_HEADER, &metadata.transfer_syntax),
            (SOP_CLASS_UID_HEADER, &metadata.sop_class_uid),
            (SOP_INSTANCE_UID_HEADER, &metadata.sop_instance_uid),
        ] {
            Self::check_stated_header(headers, header, actual.as_deref())?;
        }
        Ok(dataset)
    }

    /// Reject a request whose `header` states a value other than `actual`,
    /// the value read from the instance. An absent header states nothing.
    fn check_stated_header(
        headers: &HashMap<String, String>,
        header: &str,
        actual: Option<&str>,
    ) -> Result<(), Error> {
        let Some(stated) = headers.get(header) else {
            return Ok(());
        };
        // HTTP strips whitespace around a header value, so the instance's
        // value is trimmed the same way before the two are compared.
        let stated = stated.trim();
        let actual = actual.map(str::trim);
        if Some(stated) == actual {
            return Ok(());
        }
        // Both values come from the request. `{:?}` escapes control
        // characters so neither can forge a log line.
        Err(Error::from(format!(
            "dicom_scu_streaming store: header {} says {:?} but the instance says {:?}",
            header, stated, actual
        )))
    }

    /// Build the final response envelope for a C-STORE result.
    ///
    /// The DIMSE SCP-side dispatcher (`run_streaming_response`) keeps only the
    /// LAST envelope from the response stream and inspects
    /// `metadata.dicom_status`. A `0x0000` value means success. A `0xC000` value
    /// signals failure and the SCP returns a DIMSE failure status to the client.
    ///
    /// Metadata does not reach a dispatcher on another Harmony. Over HTTP only
    /// the body is sent, so the `success` key in the body is the result that
    /// dispatcher reads. Keep it.
    pub(crate) fn build_store_response(
        request_details: &crate::models::envelope::envelope::RequestDetails,
        command: &Option<Command>,
        remote_node: &RemoteNode,
        success: bool,
        error: Option<String>,
        sop_class_uid: Option<String>,
        sop_instance_uid: Option<String>,
    ) -> ResponseEnvelope<Vec<u8>> {
        let status_code: u16 = if success { 200 } else { 500 };
        let dicom_status = if success { "0x0000" } else { "0xC000" };

        let mut envelope_metadata = HashMap::new();
        envelope_metadata.insert("stream_sequence".to_string(), "1".to_string());
        envelope_metadata.insert("dicom_status".to_string(), dicom_status.to_string());
        envelope_metadata.insert(
            "store_result".to_string(),
            if success { "success".to_string() } else { "failure".to_string() },
        );
        if let Some(ref uid) = sop_class_uid {
            envelope_metadata.insert("sop_class_uid".to_string(), uid.clone());
        }
        if let Some(ref uid) = sop_instance_uid {
            envelope_metadata.insert("sop_instance_uid".to_string(), uid.clone());
        }

        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let mut body = serde_json::json!({
            "operation": "store",
            "success": success,
            "remote_aet": remote_node.ae_title,
            "host": remote_node.host,
            "port": remote_node.port,
        });
        if let Some(uid) = sop_class_uid {
            body["sop_class_uid"] = serde_json::json!(uid);
        }
        if let Some(uid) = sop_instance_uid {
            body["sop_instance_uid"] = serde_json::json!(uid);
        }
        if let Some(err) = error {
            body["error"] = serde_json::json!(err);
        }

        let body_bytes = serde_json::to_vec(&body).unwrap_or_default();

        let mut response = ResponseEnvelope::from_backend_with_command(
            request_details.clone(),
            status_code,
            headers,
            body_bytes,
            Some(envelope_metadata),
            command.clone(),
        );
        response.normalized_data = Some(body);
        response
            .response_details
            .metadata
            .insert("dicom_status".to_string(), dicom_status.to_string());
        response
    }
}

#[async_trait]
impl ServiceType for DicomScuStreamingBackend {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate remote connection parameters
        self.create_remote_node(options)?;
        Self::storage_transfer_syntaxes(options)?;
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

        let storage_transfer_syntaxes = Self::storage_transfer_syntaxes(options)
            .map_err(|e| Error::from(format!("Invalid transfer_syntaxes option: {:?}", e)))?;
        let dimse_config = DimseConfig {
            local_aet: local_aet.clone(),
            storage_transfer_syntaxes,
            ..Default::default()
        };

        let scu = DimseScu::new(dimse_config);

        let op_normalized = Self::resolve_streaming_op(&envelope)?;

        // A store body holds a whole instance and has no query in it, so it
        // is not parsed for query parameters.
        let query_params = if op_normalized == "store" {
            HashMap::new()
        } else {
            self.extract_query_params(&envelope)?
        };

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
            "store" => {
                // Extract the single DICOM instance from the request envelope.
                // The SCP-side dispatcher (`run_streaming_response`) invokes this
                // arm once per C-STORE PDU and consumes the resulting response
                // stream down to its last envelope, from which it reads
                // `dicom_status` to decide the DIMSE status returned to the
                // originating SCU.
                let dataset = Self::extract_store_dataset(&envelope)?;
                let sop_class_uid = dataset.metadata().sop_class_uid.clone();
                let sop_instance_uid = dataset.metadata().sop_instance_uid.clone();

                info!(
                    "dicom_scu_streaming store: forwarding instance (size_bytes={:?}) to {}@{}:{} (sop_instance_uid={:?}, transfer_syntax={:?})",
                    dataset.metadata().size_bytes,
                    remote_node.ae_title,
                    remote_node.host,
                    remote_node.port,
                    sop_instance_uid,
                    dataset.metadata().transfer_syntax,
                );

                let request_details = envelope.request_details.clone();
                let command = envelope.command.clone();
                let remote_node_for_response = remote_node.clone();

                let store_future = async move {
                    let (success, error) = match scu.store(&remote_node, dataset).await {
                        Ok(true) => {
                            debug!("dicom_scu_streaming store: C-STORE succeeded");
                            (true, None)
                        }
                        Ok(false) => {
                            warn!("dicom_scu_streaming store: C-STORE returned false");
                            (false, Some("SCU reported C-STORE failure".to_string()))
                        }
                        Err(e) => {
                            let msg = format!(
                                "C-STORE to {}@{}:{} failed: {}",
                                remote_node.ae_title, remote_node.host, remote_node.port, e
                            );
                            warn!("dicom_scu_streaming store: {}", msg);
                            (false, Some(msg))
                        }
                    };

                    Ok(Self::build_store_response(
                        &request_details,
                        &command,
                        &remote_node_for_response,
                        success,
                        error,
                        sop_class_uid,
                        sop_instance_uid,
                    ))
                };

                Ok(Box::pin(stream::once(store_future)))
            }
            _ => Err(Error::from(format!(
                "Unsupported streaming operation: '{}'. Supported: get, find, store",
                op_normalized
            ))),
        }
    }
}

impl DicomScuStreamingBackend {
    /// The DIMSE operation this request asks for, lower-cased.
    ///
    /// Order: the typed `Command`, then the `x-dimse-op` header that
    /// `dicom_to_http` sets between Harmony instances, then `"get"`. A
    /// `Command` with no DIMSE service keeps its own name, so the caller
    /// rejects it instead of defaulting to C-GET.
    ///
    /// Errors when the envelope still carries the removed `dimse_op` metadata
    /// key. A pipeline that pinned the operation that way would otherwise fall
    /// back to the client-supplied header without anyone noticing.
    fn resolve_streaming_op(envelope: &RequestEnvelope<Vec<u8>>) -> Result<String, Error> {
        super::dicom::reject_dimse_op_metadata(envelope, "dicom_scu_streaming")?;

        let op = match envelope.resolve_operation() {
            Some(operation) => operation
                .dimse_op()
                .map(str::to_string)
                .unwrap_or_else(|| operation.to_string()),
            None => envelope
                .request_details
                .headers
                .get("x-dimse-op")
                .cloned()
                .unwrap_or_else(|| "get".to_string()),
        };

        Ok(op.trim_start_matches('/').to_lowercase())
    }

    /// One retrieved instance as a response envelope: the Part 10 bytes as the
    /// body with an `application/dicom` content type, its DICOM JSON as the
    /// normalized data for middleware, and the SOP UIDs as headers and metadata.
    fn instance_envelope(
        bytes: Vec<u8>,
        object: &dicom_object::InMemDicomObject,
        metadata: &dimse::types::DatasetMetadata,
        sequence: u64,
        request_details: &crate::models::envelope::envelope::RequestDetails,
        command: &Option<Command>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        let json = dicom_json_tool::identifier_to_json_value(object)
            .map_err(|e| Error::from(format!("Failed to convert DICOM to JSON: {}", e)))?;

        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/dicom".to_string());
        if let Some(ref sop_instance_uid) = metadata.sop_instance_uid {
            headers.insert("x-dicom-sop-instance-uid".to_string(), sop_instance_uid.clone());
        }
        if let Some(ref sop_class_uid) = metadata.sop_class_uid {
            headers.insert("x-dicom-sop-class-uid".to_string(), sop_class_uid.clone());
        }

        let mut envelope_metadata = HashMap::new();
        envelope_metadata.insert("stream_sequence".to_string(), sequence.to_string());
        envelope_metadata.insert("instance_metadata".to_string(), "true".to_string());
        if let Some(ref sop_instance_uid) = metadata.sop_instance_uid {
            envelope_metadata.insert("sop_instance_uid".to_string(), sop_instance_uid.clone());
        }

        let mut response = ResponseEnvelope::from_backend_with_command(
            request_details.clone(),
            200,
            headers,
            bytes,
            Some(envelope_metadata),
            command.clone(),
        );
        response.normalized_data = Some(json);
        Ok(response)
    }

    /// Convert a DIMSE DatasetStream to a stream of ResponseEnvelopes
    pub(crate) fn convert_dimse_stream_to_envelopes(
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

                    let bytes = std::fs::read(path)
                        .map_err(|e| Error::from(format!("Failed to read DICOM file: {}", e)))?;
                    let object = dicom_object::open_file(path)
                        .map_err(|e| Error::from(format!("Failed to parse DICOM file: {}", e)))?;

                    Self::instance_envelope(bytes, &object, metadata, sequence, &request_details, &command)
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
                Ok(ref dataset @ DatasetStream::Object { ref object, ref metadata }) => {
                    // The native C-GET SCU hands instances over in memory. They go out
                    // in the same shape as file-backed ones so the wire format does not
                    // depend on which SCU produced them.
                    debug!(
                        "Processing in-memory DICOM instance {}",
                        metadata.sop_instance_uid.as_deref().unwrap_or("<unknown>")
                    );

                    let bytes = dataset
                        .to_part10_bytes()
                        .map_err(|e| Error::from(format!("Failed to encode DICOM instance: {}", e)))?
                        .to_vec();

                    Self::instance_envelope(bytes, object, metadata, sequence, &request_details, &command)
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

    // ── store arm helpers ───────────────────────────────────────────────────
    //
    // The store arm dispatches to the external DIMSE SCU. Live SCU verification
    // needs an integration harness (a running SCP peer) that this crate does not
    // ship. The tests below exercise the deterministic pieces: input extraction
    // for the two accepted payload shapes, rejection of everything else, and the response
    // envelope contract that the DIMSE SCP-side dispatcher relies on
    // (dicom_status metadata plus status code).

    fn make_part10_bytes(sop_instance_uid: &str) -> Vec<u8> {
        make_part10_bytes_with_transfer_syntax(sop_instance_uid, "1.2.840.10008.1.2.1")
    }

    fn make_part10_bytes_with_transfer_syntax(
        sop_instance_uid: &str,
        transfer_syntax_uid: &str,
    ) -> Vec<u8> {
        use dicom_object::{FileDicomObject, InMemDicomObject};

        // Build a minimal file meta table so `from_reader` accepts the bytes.
        let mut obj = InMemDicomObject::new_empty();
        let _ = obj.put(dicom_core::DataElement::new(
            dicom_dictionary_std::tags::SOP_CLASS_UID,
            dicom_core::VR::UI,
            dicom_core::PrimitiveValue::from("1.2.840.10008.5.1.4.1.1.7"),
        ));
        let _ = obj.put(dicom_core::DataElement::new(
            dicom_dictionary_std::tags::SOP_INSTANCE_UID,
            dicom_core::VR::UI,
            dicom_core::PrimitiveValue::from(sop_instance_uid),
        ));

        let file_meta = dicom_object::FileMetaTableBuilder::new()
            .transfer_syntax(transfer_syntax_uid)
            .media_storage_sop_class_uid("1.2.840.10008.5.1.4.1.1.7")
            .media_storage_sop_instance_uid(sop_instance_uid)
            .build()
            .expect("file meta build");

        let file_obj = FileDicomObject::new_empty_with_meta(file_meta);
        let mut file_obj = file_obj;
        for elem in obj.into_iter() {
            file_obj.put(elem);
        }

        let mut buf: Vec<u8> = Vec::new();
        file_obj
            .write_all(&mut buf)
            .expect("write part10 to buffer");
        buf
    }

    fn envelope_with_original_data(data: Vec<u8>) -> RequestEnvelope<Vec<u8>> {
        crate::models::envelope::envelope::RequestEnvelope::builder()
            .method("POST".to_string())
            .uri("dimse://scp/store".to_string())
            .headers(HashMap::new())
            .cookies(HashMap::new())
            .query_params(HashMap::new())
            .cache_status(None)
            .metadata(HashMap::new())
            .target_details(None)
            .command(None)
            .original_data(data)
            .normalized_data(None)
            .normalized_snapshot(None)
            .build()
            .expect("build envelope")
    }

    fn extract_error(envelope: &RequestEnvelope<Vec<u8>>) -> String {
        match DicomScuStreamingBackend::extract_store_dataset(envelope) {
            Ok(_) => panic!("the store request should be rejected"),
            Err(e) => format!("{:?}", e),
        }
    }

    fn json_envelope(payload: serde_json::Value) -> RequestEnvelope<Vec<u8>> {
        envelope_with_original_data(serde_json::to_vec(&payload).expect("serialize payload"))
    }

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn test_extract_store_dataset_rejects_empty_payload() {
        let msg = extract_error(&envelope_with_original_data(Vec::new()));
        assert!(msg.contains("no original_data"), "unexpected error: {}", msg);
    }

    #[test]
    fn test_extract_store_dataset_never_reads_a_file_named_by_the_request() {
        // AURA-2408. The file is a valid instance, so the only way this
        // request can fail is that the backend did not open it.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("instance.dcm");
        std::fs::write(&file, make_part10_bytes("1.2.3.4.99.1")).expect("write instance");

        let msg = extract_error(&json_envelope(serde_json::json!({
            "command": {"message_id": 1},
            "dataset": {},
            "file": file.to_string_lossy(),
        })));

        assert!(msg.contains("does not read files"), "unexpected error: {}", msg);
    }

    #[test]
    fn test_extract_store_dataset_prefers_part10_b64_and_ignores_file() {
        // The DIMSE adapter sends both keys to a backend in its own pipeline.
        // The path does not exist, so reading it would fail the request.
        let uid = "1.2.3.4.99.3";
        let envelope = json_envelope(serde_json::json!({
            "file": "/nonexistent/aura-2408/instance.dcm",
            "part10_b64": b64(&make_part10_bytes(uid)),
            "transfer_syntax_uid": "1.2.840.10008.1.2.1",
        }));

        let dataset = DicomScuStreamingBackend::extract_store_dataset(&envelope).expect("extract");

        assert_eq!(dataset.metadata().sop_instance_uid.as_deref(), Some(uid));
        assert_eq!(
            dataset.metadata().sop_class_uid.as_deref(),
            Some("1.2.840.10008.5.1.4.1.1.7")
        );
        assert_eq!(
            dataset.metadata().transfer_syntax.as_deref(),
            Some("1.2.840.10008.1.2.1")
        );
    }

    #[test]
    fn test_extract_store_dataset_accepts_part10_body() {
        let uid = "1.2.3.4.99.2";
        let bytes = make_part10_bytes(uid);
        let envelope = envelope_with_original_data(bytes.clone());

        let dataset = DicomScuStreamingBackend::extract_store_dataset(&envelope).expect("extract");

        assert_eq!(dataset.metadata().sop_instance_uid.as_deref(), Some(uid));
        assert_eq!(
            dataset.metadata().transfer_syntax.as_deref(),
            Some("1.2.840.10008.1.2.1")
        );
        assert_eq!(dataset.metadata().size_bytes, Some(bytes.len() as u64));
    }

    #[test]
    fn test_extract_store_dataset_reads_a_transfer_syntax_that_is_not_the_default() {
        let implicit_vr_le = "1.2.840.10008.1.2";
        let uid = "1.2.3.4.99.8";
        let envelope = envelope_with_original_data(make_part10_bytes_with_transfer_syntax(
            uid,
            implicit_vr_le,
        ));

        let dataset = DicomScuStreamingBackend::extract_store_dataset(&envelope).expect("extract");

        assert_eq!(dataset.metadata().transfer_syntax.as_deref(), Some(implicit_vr_le));
        assert_eq!(dataset.metadata().sop_instance_uid.as_deref(), Some(uid));
    }

    #[tokio::test]
    async fn test_extract_store_dataset_keeps_sample_instance_pixel_data() {
        let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/dicom/study_1/series_1/CT.1.1.dcm");
        let bytes = std::fs::read(&source_path).expect("read sample instance");
        let source = dicom_object::open_file(&source_path).expect("open sample instance");

        let dataset = DicomScuStreamingBackend::extract_store_dataset(&json_envelope(
            serde_json::json!({ "part10_b64": b64(&bytes) }),
        ))
        .expect("extract");
        let received = dataset.to_object().await.expect("dataset object");

        assert_eq!(
            dataset.metadata().transfer_syntax.as_deref(),
            Some(source.meta().transfer_syntax())
        );
        let pixels = |object: &dicom_object::InMemDicomObject| {
            object
                .element_by_name("PixelData")
                .expect("pixel data element")
                .to_bytes()
                .expect("pixel bytes")
                .to_vec()
        };
        assert_eq!(pixels(&received), pixels(&source));
    }

    #[test]
    fn test_extract_store_dataset_rejects_body_that_is_not_part10_or_json() {
        let msg = extract_error(&envelope_with_original_data(
            b"raw dataset bytes without DICM magic".to_vec(),
        ));
        assert!(msg.contains("neither a DICOM Part 10 file nor JSON"), "unexpected error: {}", msg);
    }

    #[test]
    fn test_extract_store_dataset_rejects_json_without_part10_b64() {
        // `body_b64` is the executor's key for a binary body. It is not a
        // way to hand this backend an instance.
        for payload in [
            serde_json::json!({ "dataset": {} }),
            serde_json::json!({ "body_b64": b64(&make_part10_bytes("1.2.3.4.99.7")) }),
        ] {
            let msg = extract_error(&json_envelope(payload));
            assert!(msg.contains("no 'part10_b64' string"), "unexpected error: {}", msg);
        }
    }

    #[test]
    fn test_extract_store_dataset_names_this_backend_in_its_error() {
        // The reason comes from the shared body parser. The prefix is added
        // here, and operators search the log for it.
        let msg = extract_error(&json_envelope(serde_json::json!({ "dataset": {} })));
        let expected = "dicom_scu_streaming store: the JSON body has no 'part10_b64' string. \
             Send the DICOM Part 10 file as the request body, \
             or as base64 in the 'part10_b64' key of a JSON body";
        // `extract_error` formats the error with `{:?}`.
        assert_eq!(msg, format!("{:?}", expected));
    }

    #[test]
    fn test_extract_store_dataset_rejects_invalid_base64() {
        let msg = extract_error(&json_envelope(serde_json::json!({ "part10_b64": "not base64!" })));
        assert!(msg.contains("not valid base64"), "unexpected error: {}", msg);
    }

    #[test]
    fn test_extract_store_dataset_rejects_part10_b64_that_is_not_part10() {
        let msg = extract_error(&json_envelope(serde_json::json!({
            "part10_b64": b64(b"raw dataset bytes without DICM magic"),
        })));
        assert!(msg.contains("not a DICOM Part 10 file"), "unexpected error: {}", msg);
    }

    #[test]
    fn test_extract_store_dataset_rejects_part10_magic_without_a_dataset() {
        let mut bytes = vec![0u8; 128];
        bytes.extend_from_slice(b"DICM");
        bytes.extend_from_slice(&[0u8; 100]);

        let msg = extract_error(&envelope_with_original_data(bytes));
        assert!(msg.contains("not a readable DICOM Part 10 file"), "unexpected error: {}", msg);
    }

    #[test]
    fn test_extract_store_dataset_checks_transfer_syntax_header_against_file() {
        let bytes = make_part10_bytes("1.2.3.4.99.4");

        let mut matching = envelope_with_original_data(bytes.clone());
        matching.request_details.headers.insert(
            TRANSFER_SYNTAX_HEADER.to_string(),
            "1.2.840.10008.1.2.1".to_string(),
        );
        DicomScuStreamingBackend::extract_store_dataset(&matching)
            .expect("a header that matches the file is accepted");

        let mut contradicting = envelope_with_original_data(bytes);
        contradicting.request_details.headers.insert(
            TRANSFER_SYNTAX_HEADER.to_string(),
            "1.2.840.10008.1.2".to_string(),
        );
        let msg = extract_error(&contradicting);
        assert!(msg.contains(TRANSFER_SYNTAX_HEADER), "unexpected error: {}", msg);
        assert!(msg.contains("1.2.840.10008.1.2\\\" but the instance"), "unexpected error: {}", msg);
    }

    #[test]
    fn test_extract_store_dataset_checks_sop_uid_headers_against_instance() {
        let uid = "1.2.3.4.99.5";
        let bytes = make_part10_bytes(uid);
        let with_headers = |class_uid: &str, instance_uid: &str| {
            let mut envelope = envelope_with_original_data(bytes.clone());
            let headers = &mut envelope.request_details.headers;
            headers.insert(SOP_CLASS_UID_HEADER.to_string(), class_uid.to_string());
            headers.insert(SOP_INSTANCE_UID_HEADER.to_string(), instance_uid.to_string());
            envelope
        };

        DicomScuStreamingBackend::extract_store_dataset(&with_headers(
            "1.2.840.10008.5.1.4.1.1.7",
            uid,
        ))
        .expect("headers that match the instance are accepted");

        let msg = extract_error(&with_headers("1.2.840.10008.5.1.4.1.1.7", "1.2.3.4.99.6"));
        assert!(msg.contains(SOP_INSTANCE_UID_HEADER), "unexpected error: {}", msg);

        let msg = extract_error(&with_headers("1.2.840.10008.5.1.4.1.1.2", uid));
        assert!(msg.contains(SOP_CLASS_UID_HEADER), "unexpected error: {}", msg);
    }

    #[test]
    fn test_build_store_response_success_sets_dicom_status_zero() {
        let node = RemoteNode::new("PACS".to_string(), "127.0.0.1".to_string(), 4242);
        let request_details = dummy_request_details();
        let command = Some(Command::new(Operation::Store));

        let env = DicomScuStreamingBackend::build_store_response(
            &request_details,
            &command,
            &node,
            true,
            None,
            Some("1.2.840.10008.5.1.4.1.1.7".to_string()),
            Some("1.2.3.4.99.9".to_string()),
        );

        assert_eq!(env.response_details.status, 200);
        assert_eq!(
            env.response_details.metadata.get("dicom_status").map(|s| s.as_str()),
            Some("0x0000"),
        );
        assert_eq!(
            env.response_details.metadata.get("sop_instance_uid").map(|s| s.as_str()),
            Some("1.2.3.4.99.9"),
        );
    }

    #[test]
    fn test_build_store_response_failure_sets_dicom_status_c000() {
        let node = RemoteNode::new("PACS".to_string(), "127.0.0.1".to_string(), 4242);
        let request_details = dummy_request_details();

        let env = DicomScuStreamingBackend::build_store_response(
            &request_details,
            &None,
            &node,
            false,
            Some("association failed".to_string()),
            None,
            None,
        );

        assert_eq!(env.response_details.status, 500);
        assert_eq!(
            env.response_details.metadata.get("dicom_status").map(|s| s.as_str()),
            Some("0xC000"),
        );
        // A sending Harmony reads the result from the body.
        let body = env.normalized_data.expect("the response has a JSON body");
        assert_eq!(body.get("success"), Some(&serde_json::json!(false)));
        assert_eq!(body.get("error"), Some(&serde_json::json!("association failed")));
    }

    #[tokio::test]
    async fn test_store_arm_rejects_unsupported_op_message_lists_store() {
        // Sanity check on the fallback error message. The store arm is now supported,
        // so the fallback message must advertise it alongside get and find.
        let backend = make_backend(Some("REMOTE"), Some("127.0.0.1"), Some(4242));

        // Envelope carrying an unknown op via the bridging header.
        let mut headers = HashMap::new();
        headers.insert("x-dimse-op".to_string(), "unknown_op".to_string());

        let envelope = crate::models::envelope::envelope::RequestEnvelope::builder()
            .method("POST".to_string())
            .uri("dimse://scp/unknown_op".to_string())
            .headers(headers)
            .cookies(HashMap::new())
            .query_params(HashMap::new())
            .cache_status(None)
            .metadata(HashMap::new())
            .target_details(None)
            .command(None)
            .original_data(Vec::new())
            .normalized_data(None)
            .normalized_snapshot(None)
            .build()
            .expect("build envelope");

        let options = make_options(&[
            ("aet", serde_json::json!("REMOTE")),
            ("host", serde_json::json!("127.0.0.1")),
            ("port", serde_json::json!(4242)),
        ]);

        let result = backend.backend_outgoing_stream(envelope, &options).await;
        let err = match result {
            Ok(_) => panic!("unknown op should fail"),
            Err(e) => e,
        };
        let msg = format!("{:?}", err);
        assert!(msg.contains("store"), "error must advertise 'store': {}", msg);
        assert!(msg.contains("find"));
        assert!(msg.contains("get"));
    }

    #[tokio::test]
    async fn test_command_without_dimse_service_is_rejected_over_header() {
        // The typed Command wins over the x-dimse-op header. An operation with
        // no DIMSE service must fail, not fall back to the header or to "get".
        let backend = make_backend(Some("REMOTE"), Some("127.0.0.1"), Some(4242));

        let mut headers = HashMap::new();
        headers.insert("x-dimse-op".to_string(), "get".to_string());

        let envelope = crate::models::envelope::envelope::RequestEnvelope::builder()
            .method("POST".to_string())
            .uri("dimse://scp/delete".to_string())
            .headers(headers)
            .cookies(HashMap::new())
            .query_params(HashMap::new())
            .cache_status(None)
            .metadata(HashMap::new())
            .target_details(None)
            .command(Some(Command::new(Operation::Delete)))
            .original_data(Vec::new())
            .normalized_data(None)
            .normalized_snapshot(None)
            .build()
            .expect("build envelope");

        let options = make_options(&[
            ("aet", serde_json::json!("REMOTE")),
            ("host", serde_json::json!("127.0.0.1")),
            ("port", serde_json::json!(4242)),
        ]);

        let result = backend.backend_outgoing_stream(envelope, &options).await;
        let err = match result {
            Ok(_) => panic!("an operation with no DIMSE service should fail"),
            Err(e) => e,
        };
        let msg = format!("{:?}", err);
        assert!(msg.contains("'delete'"), "error must name the rejected operation: {}", msg);
    }

    // ── resolve_streaming_op ─────────────────────────────────────────────────

    fn op_envelope(
        command: Option<Command>,
        header: Option<&str>,
        request_meta: &[(&str, &str)],
    ) -> RequestEnvelope<Vec<u8>> {
        let mut headers = HashMap::new();
        if let Some(op) = header {
            headers.insert("x-dimse-op".to_string(), op.to_string());
        }
        let metadata: HashMap<String, String> = request_meta
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        crate::models::envelope::envelope::RequestEnvelope::builder()
            .method("POST".to_string())
            .uri("/".to_string())
            .headers(headers)
            .cookies(HashMap::new())
            .query_params(HashMap::new())
            .cache_status(None)
            .metadata(metadata)
            .target_details(None)
            .command(command)
            .original_data(Vec::new())
            .normalized_data(None)
            .normalized_snapshot(None)
            .build()
            .expect("build envelope")
    }

    #[test]
    fn test_resolve_op_uses_command_before_header() {
        let env = op_envelope(Some(Command::new(Operation::Query)), Some("store"), &[]);
        assert_eq!(DicomScuStreamingBackend::resolve_streaming_op(&env).unwrap(), "find");
    }

    #[test]
    fn test_resolve_op_falls_back_to_header_without_command() {
        let env = op_envelope(None, Some("/FIND"), &[]);
        assert_eq!(DicomScuStreamingBackend::resolve_streaming_op(&env).unwrap(), "find");
    }

    #[test]
    fn test_resolve_op_defaults_to_get() {
        let env = op_envelope(None, None, &[]);
        assert_eq!(DicomScuStreamingBackend::resolve_streaming_op(&env).unwrap(), "get");
    }

    #[test]
    fn test_resolve_op_rejects_dimse_op_in_request_metadata() {
        let env = op_envelope(None, Some("store"), &[("dimse_op", "find")]);
        let err = DicomScuStreamingBackend::resolve_streaming_op(&env).unwrap_err();
        let msg = format!("{:?}", err);
        assert!(msg.contains("request_details.metadata.dimse_op"), "{}", msg);
        assert!(msg.contains("command"), "{}", msg);
    }

    #[test]
    fn test_resolve_op_rejects_dimse_op_in_target_metadata() {
        // A transform that pinned the operation in target metadata must not
        // fall through to the client-supplied header.
        let mut env = op_envelope(None, Some("store"), &[]);
        env.set_target_metadata("dimse_op", "find");
        let err = DicomScuStreamingBackend::resolve_streaming_op(&env).unwrap_err();
        let msg = format!("{:?}", err);
        assert!(msg.contains("target_details.metadata.dimse_op"), "{}", msg);
    }
}
