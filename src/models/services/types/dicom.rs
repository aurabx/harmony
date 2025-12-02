use crate::config::config::ConfigError;
use crate::models::connection::ConnectionConfig;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::globals::get_storage;
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use dicom_json_tool as djt;
use dimse::types::{FindQuery, GetQuery, QueryLevel};
use dimse::{DimseConfig, DimseScu, RemoteNode};
use std::fs;
use std::path::Path;
use tracing::warn;
use uuid::Uuid;

/// DICOM SCU (Service Class User) backend configuration
///
/// This service handles outgoing DICOM DIMSE requests (C-ECHO, C-FIND, C-MOVE, C-GET)
/// to remote PACS systems.
#[derive(Debug, Deserialize)]
pub struct DicomScuBackend {
    pub local_aet: Option<String>,
    pub aet: Option<String>, // Remote AET
    pub host: Option<String>,
    pub port: Option<u16>,
    pub use_tls: Option<bool>,
}

impl DicomScuBackend {
    /// Get the local AET from options or struct, with default fallback
    fn get_local_aet(&self, options: &HashMap<String, Value>) -> Option<String> {
        options
            .get("local_aet")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.local_aet.clone())
            .or_else(|| Some("HARMONY_DICOM".to_string()))
    }

    /// Create a remote node from configuration
    fn create_remote_node(
        &self,
        options: &HashMap<String, Value>,
    ) -> Result<RemoteNode, ConfigError> {
        // Parse connection config if present
        let connection = options
            .get("connection")
            .and_then(|v| serde_json::from_value::<ConnectionConfig>(v.clone()).ok());

        let aet = options
            .get("aet")
            .and_then(|v| v.as_str())
            .or(self.aet.as_deref())
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom_scu".to_string(),
                reason: "Missing remote 'aet' (Application Entity Title)".to_string(),
            })?
            .to_string();

        let host = options
            .get("host")
            .and_then(|v| v.as_str())
            .or(self.host.as_deref())
            .or_else(|| connection.as_ref().map(|c| c.host.as_str()))
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom_scu".to_string(),
                reason: "Missing 'host' (DICOM server address)".to_string(),
            })?
            .to_string();

        let port = options
            .get("port")
            .and_then(|v| v.as_u64())
            .or(self.port.map(|p| p as u64))
            .or_else(|| connection.as_ref().and_then(|c| c.port.map(|p| p as u64)))
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom_scu".to_string(),
                reason: "Missing 'port'".to_string(),
            })?;

        // DICOM servers commonly use privileged ports like 104, so allow 1-65535 for remote nodes
        if !(1..=65535).contains(&port) {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom_scu".to_string(),
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
}

#[async_trait]
impl ServiceType for DicomScuBackend {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // DicomScuBackend is always a backend - validate remote connection parameters
        self.create_remote_node(options)?;

        // Validate dimse_retrieve_mode option if provided
        if let Some(retrieve_mode) = options.get("dimse_retrieve_mode") {
            if let Some(mode_str) = retrieve_mode.as_str() {
                let mode_lower = mode_str.to_lowercase();
                if !matches!(mode_lower.as_str(), "get" | "move") {
                    return Err(ConfigError::InvalidEndpoint {
                        name: "dicom_scu".to_string(),
                        reason: "dimse_retrieve_mode must be either 'get' or 'move'".to_string(),
                    });
                }
            } else {
                return Err(ConfigError::InvalidEndpoint {
                    name: "dicom_scu".to_string(),
                    reason: "dimse_retrieve_mode must be a string value".to_string(),
                });
            }
        }

        Ok(())
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Backend usage only - no HTTP routes (DIMSE SCU operates at protocol level)
        vec![]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        use crate::models::envelope::envelope::RequestEnvelope;
        use crate::utils::Error;
        use std::collections::HashMap as Map;

        if ctx.protocol != crate::models::protocol::Protocol::Dimse {
            return Err(Error::from(
                "DicomEndpoint only supports Protocol::Dimse in build_protocol_envelope",
            ));
        }

        // Build minimal RequestDetails using meta
        let metadata: Map<String, String> = ctx.meta.clone();
        let op = metadata
            .get("operation")
            .cloned()
            .unwrap_or_else(|| "DIMSE".into());
        let uri = format!("mock-dicom://scp/{}", op.to_lowercase());

        // Prefer normalized_data as the JSON body if payload is JSON
        let normalized: Option<serde_json::Value> = serde_json::from_slice(&ctx.payload).ok();

        RequestEnvelope::builder()
            .method(op)
            .uri(uri)
            .headers(Map::new())
            .cookies(Map::new())
            .query_params(Map::new())
            .cache_status(None)
            .metadata(metadata)
            .target_details(None)
            .original_data(ctx.payload)
            .normalized_data(normalized)
            .normalized_snapshot(None)
            .build()
    }
}

#[async_trait]
impl ServiceHandler<Value> for DicomScuBackend {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        _envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // DicomScuBackend is not an endpoint - this should never be called
        Err(Error::from(
            "DicomScuBackend cannot be used as an endpoint (use dicom_scp instead)",
        ))
    }

    async fn backend_outgoing_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // Backend usage - perform DIMSE SCU operations
        envelope = self
            .handle_backend_request(&mut envelope, options)
            .await
            .expect("DICOM response failed");

        // Detect error conditions and set appropriate HTTP status
        let status = if let Some(ref normalized) = envelope.normalized_data {
            if let Some(error) = normalized.get("error") {
                if error.as_str() == Some("Study not found") {
                    404
                } else {
                    500
                }
            } else if let Some(success) = normalized.get("success") {
                if success.as_bool() == Some(false) {
                    500 // DICOM operation failed
                } else {
                    200
                }
            } else {
                200
            }
        } else {
            200
        };

        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let body = if let Some(ref normalized) = envelope.normalized_data {
            serde_json::to_vec(normalized).unwrap_or_default()
        } else {
            Vec::new()
        };

        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            headers,
            body,
            None,
        );

        response_envelope.normalized_data = envelope.normalized_data;

        Ok(response_envelope)
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
            .insert("service".to_string(), "dicom".to_string());
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Build response from ResponseEnvelope
        let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        // Add headers from response_details
        for (k, v) in &envelope.response_details.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        // Use original_data if available, otherwise serialize normalized_data
        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else if let Some(normalized) = envelope.normalized_data {
            let body_bytes = serde_json::to_vec(&normalized)
                .map_err(|_| Error::from("Failed to serialize DICOM response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct DICOM HTTP response"))
    }
}

impl DicomScuBackend {
    /// Handle backend (SCU) request processing
    async fn handle_backend_request(
        &self,
        envelope: &mut RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Create remote node configuration
        let remote_node = self
            .create_remote_node(options)
            .map_err(|e| Error::from(format!("Failed to create remote node: {:?}", e)))?;

        // Create DIMSE SCU configuration
        let local_aet = self
            .get_local_aet(options)
            .unwrap_or_else(|| "HARMONY_SCU".to_string());

        let mut dimse_config = DimseConfig {
            local_aet,
            ..Default::default()
        };

        // If persistent Store SCP is requested, instruct SCU not to spawn a transient +P listener
        let persistent_scp = options
            .get("persistent_store_scp")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if persistent_scp {
            dimse_config.external_store_scp = true;
        }

        // Allow configuring incoming_store_port for C-MOVE via backend options
        if let Some(port_val) = options.get("incoming_store_port").and_then(|v| v.as_u64()) {
            if (1..=65535).contains(&port_val) {
                dimse_config.incoming_store_port = port_val as u16;
            }
        }

        // Create SCU client
        let scu = DimseScu::new(dimse_config);

        // Resolve DIMSE operation with proper precedence:
        // 1. Check target_details.metadata["dimse_op"] (set by middleware)
        // 2. Check request_details.metadata["dimse_op"] (fallback for compatibility)
        // 3. Check dimse_retrieve_mode option (only applies to get/move operations)
        // 4. Check if path is a valid DIMSE operation name (for direct HTTP->DICOM calls)
        // 5. Default to "get" for data retrieval
        let valid_ops = ["echo", "find", "get", "move", "store"];

        let op = envelope
            .target_details
            .as_ref()
            .and_then(|td| td.metadata.get("dimse_op"))
            .cloned()
            .or_else(|| envelope.request_details.metadata.get("dimse_op").cloned())
            .or_else(|| {
                // Only use dimse_retrieve_mode for retrieval operations
                // This allows backend configuration to override default "get" with "move"
                options
                    .get("dimse_retrieve_mode")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                // Check if path is a valid DIMSE operation (for direct HTTP->DICOM calls)
                let path = envelope
                    .request_details
                    .metadata
                    .get("path")
                    .map(|s| s.trim_start_matches('/').to_lowercase())?;

                if valid_ops.contains(&path.as_str()) {
                    Some(path)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "get".to_string());

        // Validate operation is a valid DIMSE operation
        let normalized_op = op.trim_start_matches('/').to_lowercase();
        if !valid_ops.contains(&normalized_op.as_str()) {
            return Err(Error::from(format!(
                "Invalid DIMSE operation: '{}'. Valid operations are: {}",
                op,
                valid_ops.join(", ")
            )));
        }

        let result = match normalized_op.as_str() {
            "echo" => {
                // Perform C-ECHO
                match scu.echo(&remote_node).await {
                    Ok(success) => serde_json::json!({
                        "operation": "echo",
                        "success": success,
                        "remote_aet": remote_node.ae_title,
                        "host": remote_node.host,
                        "port": remote_node.port
                    }),
                    Err(e) => serde_json::json!({
                        "operation": "echo",
                        "success": false,
                        "error": e.to_string()
                    }),
                }
            }
            "find" => {
                // Parse request body as either wrapper or raw identifier JSON
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

                // Flatten identifier JSON into tag->string map for FindQuery parameters
                let mut params: HashMap<String, String> = HashMap::new();
                if let Some(map) = identifier_json.as_object() {
                    for (tag, entry) in map.iter() {
                        // Expect { vr: ..., Value: [...] } or just { vr: ... } for return keys
                        if let Some(val_array) = entry.get("Value").and_then(|v| v.as_array()) {
                            if let Some(first) = val_array.first() {
                                if let Some(s) = first.as_str() {
                                    params.insert(tag.clone(), s.to_string());
                                } else if let Some(obj) = first.as_object() {
                                    // PN case: { Alphabetic: "..." }
                                    if let Some(alpha) =
                                        obj.get("Alphabetic").and_then(|v| v.as_str())
                                    {
                                        params.insert(tag.clone(), alpha.to_string());
                                    }
                                }
                            } else {
                                // Empty array indicates return key
                                params.insert(tag.clone(), String::new());
                            }
                        } else if entry.get("vr").is_some() {
                            // Entry has vr but no Value - treat as return key
                            params.insert(tag.clone(), String::new());
                        }
                    }
                }

                // Determine query level from metadata (explicit) or infer from parameters (fallback)
                let query_level = envelope
                    .target_details
                    .as_ref()
                    .and_then(|td| td.metadata.get("query_level"))
                    .or_else(|| envelope.request_details.metadata.get("query_level"))
                    .and_then(|level| match level.to_uppercase().as_str() {
                        "PATIENT" => Some(QueryLevel::Patient),
                        "STUDY" => Some(QueryLevel::Study),
                        "SERIES" => Some(QueryLevel::Series),
                        "IMAGE" => Some(QueryLevel::Image),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        // Fallback: infer from parameters
                        if params.get("00080018").is_some_and(|v| !v.is_empty()) {
                            QueryLevel::Image
                        } else if params.get("0020000E").is_some_and(|v| !v.is_empty()) {
                            QueryLevel::Series
                        } else if params.get("0020000D").is_some_and(|v| !v.is_empty())
                            || params.contains_key("0020000D")
                        {
                            QueryLevel::Study
                        } else {
                            QueryLevel::Study // Default to STUDY for typical FHIR ImagingStudy queries
                        }
                    });

                let mut query = FindQuery::patient(params.get("00100020").cloned()); // PatientID if present
                query.query_level = query_level;
                for (k, v) in params.into_iter() {
                    query = query.with_parameter(k, v);
                }

                // Perform C-FIND and collect results
                match scu.find(&remote_node, query).await {
                    Ok(mut stream) => {
                        use futures_util::StreamExt;
                        let mut matches: Vec<serde_json::Value> = Vec::new();
                        while let Some(item) = stream.next().await {
                            match item {
                                Ok(dimse::types::DatasetStream::File { ref path, .. }) => {
                                    if let Ok(obj) = dicom_object::open_file(path) {
                                        if let Ok(json) =
                                            dicom_json_tool::identifier_to_json_value(&obj)
                                        {
                                            matches.push(json);
                                        }
                                    }
                                }
                                Ok(dimse::types::DatasetStream::Memory { ref data, .. }) => {
                                    // Handle in-memory DICOM data
                                    if let Ok(obj) = dicom_object::from_reader(&**data) {
                                        if let Ok(json) =
                                            dicom_json_tool::identifier_to_json_value(&obj)
                                        {
                                            matches.push(json);
                                        }
                                    }
                                }
                                Ok(dimse::types::DatasetStream::Object { ref object, .. }) => {
                                    // Handle DICOM object directly
                                    if let Ok(json) =
                                        dicom_json_tool::identifier_to_json_value(object)
                                    {
                                        matches.push(json);
                                    }
                                }
                                Err(e) => {
                                    warn!("Error in dataset stream: {}", e);
                                }
                            }
                        }

                        serde_json::json!({
                            "operation": "find",
                            "success": true,
                            "matches": matches
                        })
                    }
                    Err(e) => serde_json::json!({
                        "operation": "find",
                        "success": false,
                        "error": e.to_string()
                    }),
                }
            }
            "move" => {
                // Parse request body to build a MoveQuery (destination defaults to our local AET)
                let body_json: serde_json::Value = serde_json::from_slice(&envelope.original_data)
                    .unwrap_or(serde_json::Value::Null);

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

                // Flatten identifier JSON into tag->string map for MoveQuery parameters
                let mut params: HashMap<String, String> = HashMap::new();
                if let Some(map) = identifier_json.as_object() {
                    for (tag, entry) in map.iter() {
                        if let Some(val_array) = entry.get("Value").and_then(|v| v.as_array()) {
                            if let Some(first) = val_array.first() {
                                if let Some(s) = first.as_str() {
                                    params.insert(tag.clone(), s.to_string());
                                } else if let Some(obj) = first.as_object() {
                                    if let Some(alpha) =
                                        obj.get("Alphabetic").and_then(|v| v.as_str())
                                    {
                                        params.insert(tag.clone(), alpha.to_string());
                                    }
                                }
                            } else {
                                params.insert(tag.clone(), String::new());
                            }
                        }
                    }
                }

                // Destination AE: default to our local AET (download into proxy tmp)
                let destination_aet = self
                    .get_local_aet(options)
                    .unwrap_or_else(|| "HARMONY_SCU".to_string());
                let mut move_q = dimse::types::MoveQuery::new(QueryLevel::Study, destination_aet);
                // Capture requested UID for relocation before consuming params
                let requested_uid_for_relocate = params.get("0020000D").cloned();
                for (k, v) in params.iter() {
                    move_q = move_q.with_parameter(k.clone(), v.clone());
                }

                // Preflight: ensure the requested StudyInstanceUID exists via C-FIND
                if let Some(uid) = requested_uid_for_relocate.clone() {
                    if !uid.is_empty() {
                        let mut find_q = FindQuery::patient(None);
                        find_q.query_level = QueryLevel::Study;
                        find_q = find_q.with_parameter("0020000D".to_string(), uid.clone());
                        if let Ok(mut stream) = scu.find(&remote_node, find_q).await {
                            use futures_util::StreamExt;
                            let mut any = false;
                            if let Some(_first) = stream.next().await {
                                any = true;
                            }
                            if !any {
                                // Study not found - set error in normalized_data
                                envelope.normalized_data =
                                    Some(serde_json::json!({"error": "Study not found"}));
                                // Mark to skip further backend processing
                                envelope
                                    .request_details
                                    .metadata
                                    .insert("skip_backends".into(), "true".into());
                                return Ok(envelope.clone());
                            }
                        }
                    }
                }

                // Determine storage target folder and pass to SCU if filesystem

                // Determine storage target folder and pass to SCU if filesystem
                let folder_id = Uuid::new_v4().to_string();
                let (folder_path, is_fs_backend) = if let Some(storage) = get_storage() {
                    let dir = storage
                        .ensure_dir_str(&format!("dimse/{}", folder_id))
                        .unwrap_or_else(|_| {
                            let fallback = Path::new("./tmp").join("dimse").join(&folder_id);
                            let _ = fs::create_dir_all(&fallback);
                            fallback
                        });
                    (dir, storage.is_filesystem())
                } else {
                    let base = Path::new("./tmp").join("dimse");
                    let _ = fs::create_dir_all(&base);
                    let dir = base.join(&folder_id);
                    let _ = fs::create_dir_all(&dir);
                    (dir, true)
                };

                // In persistent SCP mode, create a per-move subdirectory
                // Note: The QueryProvider now uses get_storage() directly, so no need to set a static store dir
                let mut per_move_dir_opt: Option<std::path::PathBuf> = None;
                if persistent_scp {
                    let scp_root = options
                        .get("storage_dir")
                        .and_then(|v| v.as_str())
                        .unwrap_or("./tmp/dimse");
                    let per_move_dir = std::path::Path::new(scp_root).join(&folder_id);
                    let _ = std::fs::create_dir_all(&per_move_dir);
                    per_move_dir_opt = Some(per_move_dir.clone());
                }

                match scu
                    .move_request(
                        &remote_node,
                        move_q,
                        if is_fs_backend && !persistent_scp {
                            Some(folder_path.clone())
                        } else {
                            None
                        },
                    )
                    .await
                {
                    Ok(mut stream) => {
                        use futures_util::StreamExt;
                        let mut instances: Vec<serde_json::Value> = Vec::new();
                        let mut file_count = 0usize;

                        while let Some(item) = stream.next().await {
                            if let Ok(dimse::types::DatasetStream::File { ref path, .. }) = item {
                                // For filesystem backend, files are already in folder_path.
                                // For non-filesystem, stream and persist via storage backend.
                                if !is_fs_backend {
                                    if let Some(storage) = get_storage() {
                                        let bytes = tokio::fs::read(path)
                                            .await
                                            .unwrap_or_else(|_| Vec::new());
                                        // Normalize filename to .dcm
                                        let src = Path::new(path);
                                        let base = src
                                            .file_stem()
                                            .and_then(|s| s.to_str())
                                            .unwrap_or("instance");
                                        let mut name = base.to_string();
                                        if !name.ends_with(".dcm") {
                                            name.push_str(".dcm");
                                        }
                                        let rel = format!("dimse/{}/{}", folder_id, name);
                                        let _ = storage.write_file_str(&rel, &bytes).await;
                                        // Cleanup staged file
                                        let _ = tokio::fs::remove_file(path).await;
                                    }
                                }
                                file_count += 1;

                                // Also capture identifier metadata
                                if let Ok(obj) = dicom_object::open_file(path) {
                                    if let Ok(json) =
                                        dicom_json_tool::identifier_to_json_value(&obj)
                                    {
                                        instances.push(json);
                                    }
                                }
                            }
                        }

                        // If filesystem backend, ensure .dcm extensions in-place
                        if is_fs_backend {
                            if let Ok(entries) = std::fs::read_dir(&folder_path) {
                                for e in entries.flatten() {
                                    let p = e.path();
                                    if p.is_file() {
                                        let ext = p
                                            .extension()
                                            .and_then(|e| e.to_str())
                                            .unwrap_or("")
                                            .to_lowercase();
                                        if ext != "dcm" {
                                            let mut new_p = p.clone();
                                            new_p.set_extension("dcm");
                                            let _ = std::fs::rename(&p, &new_p);
                                        }
                                    }
                                }
                            }
                        }

                        // Build response and attach folder_path/file_count
                        let mut response = serde_json::json!({
                            "operation": "move",
                            "success": true,
                            "instances": instances,
                            "folder_id": folder_id,
                            "file_count": file_count
                        });

                        if persistent_scp {
                            // In persistent mode, ensure all matching files are under per-move directory
                            let scp_root = options
                                .get("storage_dir")
                                .and_then(|v| v.as_str())
                                .unwrap_or("./tmp/dimse");
                            let per_move_dir = per_move_dir_opt
                                .clone()
                                .unwrap_or_else(|| std::path::Path::new(scp_root).join(&folder_id));
                            let _ = std::fs::create_dir_all(&per_move_dir);

                            // Extract requested StudyInstanceUID from parameters (0020000D)
                            let requested_uid =
                                requested_uid_for_relocate.clone().unwrap_or_default();
                            if !requested_uid.is_empty() {
                                // Recursively scan scp_root to find matching files, excluding the per-move directory itself
                                for entry in walkdir::WalkDir::new(scp_root)
                                    .into_iter()
                                    .filter_map(|e| e.ok())
                                {
                                    let p = entry.path();
                                    if p.is_dir() {
                                        continue;
                                    }
                                    if p.starts_with(&per_move_dir) {
                                        continue;
                                    }
                                    if let Ok(obj) = dicom_object::open_file(p) {
                                        if let Ok(json) =
                                            dicom_json_tool::identifier_to_json_value(&obj)
                                        {
                                            let uid = json
                                                .get("0020000D")
                                                .and_then(|v| v.get("Value"))
                                                .and_then(|v| v.as_array())
                                                .and_then(|arr| arr.first())
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if uid == requested_uid {
                                                let file_name = p
                                                    .file_name()
                                                    .map(|n| n.to_string_lossy().to_string())
                                                    .unwrap_or_else(|| "instance.dcm".to_string());
                                                let target = per_move_dir.join(file_name);
                                                let _ = std::fs::rename(p, &target).or_else(|_| {
                                                    std::fs::copy(p, &target)
                                                        .map(|_| std::fs::remove_file(p).ok())
                                                        .map(|_| ())
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            // Ensure .dcm extension inside per_move_dir and count files
                            let mut moved_count = 0usize;
                            if let Ok(entries) = std::fs::read_dir(&per_move_dir) {
                                for e in entries.flatten() {
                                    let p = e.path();
                                    if p.is_file() {
                                        moved_count += 1;
                                        let ext = p
                                            .extension()
                                            .and_then(|e| e.to_str())
                                            .unwrap_or("")
                                            .to_lowercase();
                                        if ext != "dcm" {
                                            let mut new_p = p.clone();
                                            new_p.set_extension("dcm");
                                            let _ = std::fs::rename(&p, &new_p);
                                        }
                                    }
                                }
                            }
                            response["folder_path"] =
                                serde_json::json!(per_move_dir.to_string_lossy());
                            response["file_count"] = serde_json::json!(moved_count);
                        } else if is_fs_backend {
                            response["folder_path"] =
                                serde_json::json!(folder_path.to_string_lossy());
                        } else {
                            // Transient mode: if no files were produced, attempt a fallback C-GET into per-move folder
                            if file_count == 0 {
                                let requested_uid =
                                    requested_uid_for_relocate.clone().unwrap_or_default();
                                if !requested_uid.is_empty() {
                                    let mut get_q = GetQuery::new(QueryLevel::Study);
                                    get_q = get_q.with_parameter(
                                        "0020000D".to_string(),
                                        requested_uid.clone(),
                                    );
                                    if let Ok(mut stream2) = scu
                                        .get_request(&remote_node, get_q, Some(folder_path.clone()))
                                        .await
                                    {
                                        use futures_util::StreamExt;
                                        let mut produced = 0usize;
                                        while let Some(item2) = stream2.next().await {
                                            if let Ok(dimse::types::DatasetStream::File {
                                                ref path,
                                                ..
                                            }) = item2
                                            {
                                                if path.is_file() {
                                                    produced += 1;
                                                }
                                            }
                                        }
                                        response["folder_path"] =
                                            serde_json::json!(folder_path.to_string_lossy());
                                        response["file_count"] = serde_json::json!(produced);
                                    }
                                }
                            }
                        }

                        response
                    }
                    Err(e) => serde_json::json!({
                        "operation": "move",
                        "success": false,
                        "error": e.to_string()
                    }),
                }
            }
            "get" => {
                // Parse request body to build a GetQuery
                let body_json: serde_json::Value = serde_json::from_slice(&envelope.original_data)
                    .unwrap_or(serde_json::Value::Null);

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

                let mut params: HashMap<String, String> = HashMap::new();
                if let Some(map) = identifier_json.as_object() {
                    for (tag, entry) in map.iter() {
                        if let Some(val_array) = entry.get("Value").and_then(|v| v.as_array()) {
                            if let Some(first) = val_array.first() {
                                if let Some(s) = first.as_str() {
                                    params.insert(tag.clone(), s.to_string());
                                } else if let Some(obj) = first.as_object() {
                                    if let Some(alpha) =
                                        obj.get("Alphabetic").and_then(|v| v.as_str())
                                    {
                                        params.insert(tag.clone(), alpha.to_string());
                                    }
                                }
                            } else {
                                params.insert(tag.clone(), String::new());
                            }
                        }
                    }
                }

                let mut get_q = GetQuery::new(QueryLevel::Study);
                for (k, v) in params.into_iter() {
                    get_q = get_q.with_parameter(k, v);
                }

                // Determine storage target folder and pass to SCU if filesystem
                let folder_id = Uuid::new_v4().to_string();
                let (folder_path, is_fs_backend) = if let Some(storage) = get_storage() {
                    let dir = storage
                        .ensure_dir_str(&format!("dimse/{}", folder_id))
                        .unwrap_or_else(|_| {
                            let fallback = Path::new("./tmp").join("dimse").join(&folder_id);
                            let _ = fs::create_dir_all(&fallback);
                            fallback
                        });
                    (dir, storage.is_filesystem())
                } else {
                    let base = Path::new("./tmp").join("dimse");
                    let _ = fs::create_dir_all(&base);
                    let dir = base.join(&folder_id);
                    let _ = fs::create_dir_all(&dir);
                    (dir, true)
                };

                match scu
                    .get_request(
                        &remote_node,
                        get_q,
                        if is_fs_backend {
                            Some(folder_path.clone())
                        } else {
                            None
                        },
                    )
                    .await
                {
                    Ok(mut stream) => {
                        use futures_util::StreamExt;
                        let mut instances: Vec<serde_json::Value> = Vec::new();
                        let mut file_count = 0usize;

                        while let Some(item) = stream.next().await {
                            if let Ok(dimse::types::DatasetStream::File { ref path, .. }) = item {
                                if !is_fs_backend {
                                    if let Some(storage) = get_storage() {
                                        let bytes = tokio::fs::read(path)
                                            .await
                                            .unwrap_or_else(|_| Vec::new());
                                        let src = Path::new(path);
                                        let base = src
                                            .file_stem()
                                            .and_then(|s| s.to_str())
                                            .unwrap_or("instance");
                                        let mut name = base.to_string();
                                        if !name.ends_with(".dcm") {
                                            name.push_str(".dcm");
                                        }
                                        let rel = format!("dimse/{}/{}", folder_id, name);
                                        let _ = storage.write_file_str(&rel, &bytes).await;
                                        let _ = tokio::fs::remove_file(path).await;
                                    }
                                }
                                file_count += 1;

                                // Also capture identifier metadata
                                if let Ok(obj) = dicom_object::open_file(path) {
                                    if let Ok(json) =
                                        dicom_json_tool::identifier_to_json_value(&obj)
                                    {
                                        instances.push(json);
                                    }
                                }
                            }
                        }

                        if is_fs_backend {
                            if let Ok(entries) = std::fs::read_dir(&folder_path) {
                                for e in entries.flatten() {
                                    let p = e.path();
                                    if p.is_file() {
                                        let ext = p
                                            .extension()
                                            .and_then(|e| e.to_str())
                                            .unwrap_or("")
                                            .to_lowercase();
                                        if ext != "dcm" {
                                            let mut new_p = p.clone();
                                            new_p.set_extension("dcm");
                                            let _ = std::fs::rename(&p, &new_p);
                                        }
                                    }
                                }
                            }
                        }

                        let mut resp = serde_json::json!({
                            "operation": "get",
                            "success": true,
                            "instances": instances,
                            "folder_id": folder_id,
                            "file_count": file_count
                        });
                        if is_fs_backend {
                            resp["folder_path"] = serde_json::json!(folder_path.to_string_lossy());
                        }
                        resp
                    }
                    Err(e) => serde_json::json!({
                        "operation": "get",
                        "success": false,
                        "error": e.to_string()
                    }),
                }
            }
            _ => {
                // This should never be reached due to validation above, but handle it gracefully
                serde_json::json!({
                    "operation": "unknown",
                    "success": false,
                    "error": format!("Unsupported DIMSE operation: '{}'. Valid operations are: echo, find, get, move, store", op)
                })
            }
        };

        envelope.normalized_data = Some(result);
        Ok(envelope.clone())
    }
}
