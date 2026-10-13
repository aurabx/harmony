use crate::config::config::ConfigError;
use crate::models::connection::ConnectionConfig;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use super::dicom_store_body::{part10_from_store_body, StoreBodyError};
use crate::globals::get_storage;
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use dicom_json_tool as djt;
use dimse::types::{DatasetStream, FindQuery, GetQuery, QueryLevel};
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

        // Derive the DIMSE operation name and URI from the typed Command.
        let command = ctx.command;
        let op = command
            .as_ref()
            .and_then(|c| c.operation.dimse_op())
            .unwrap_or("dimse");
        let uri = format!("dimse://scu/{}", op);
        let metadata: Map<String, String> = ctx.meta.clone();

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
            .command(command)
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

        let mut response_envelope = ResponseEnvelope::from_backend_with_command(
            envelope.request_details.clone(),
            status,
            headers,
            body,
            None,
            envelope.command.clone(),
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

        // Allow configuring max_pdu for DICOM associations via backend options
        if let Some(max_pdu_val) = options.get("max_pdu").and_then(|v| v.as_u64()) {
            if (16384..=131072).contains(&max_pdu_val) {
                dimse_config.max_pdu = max_pdu_val as u32;
            }
        }

        // Create SCU client
        let scu = DimseScu::new(dimse_config);

        reject_dimse_op_metadata(&envelope, "dicom_scu")?;

        // Resolve DIMSE operation — see resolve_dimse_op() for full priority order.
        let normalized_op = resolve_dimse_op(&envelope, options);

        // Validate the resolved operation
        let valid_ops = ["echo", "find", "get", "move", "store"];
        if !valid_ops.contains(&normalized_op.as_str()) {
            return Err(Error::from(format!(
                "Invalid DIMSE operation: '{}'. Valid operations are: {}",
                normalized_op,
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
                            match item {
                                Ok(dimse::types::DatasetStream::File { ref path, .. }) => {
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
                                Ok(ref dataset @ dimse::types::DatasetStream::Object { ref object, ref metadata }) => {
                                    // Without an output directory the native C-GET SCU hands
                                    // instances over in memory. Persist them under the same
                                    // storage folder the file-backed path uses.
                                    match dataset.to_part10_bytes() {
                                        Ok(bytes) => {
                                            if !is_fs_backend {
                                                if let Some(storage) = get_storage() {
                                                    let stem: String = metadata
                                                        .sop_instance_uid
                                                        .as_deref()
                                                        .unwrap_or("")
                                                        .chars()
                                                        .filter(|c| c.is_ascii_digit() || *c == '.')
                                                        .collect();
                                                    let stem = if stem.chars().any(|c| c.is_ascii_digit()) {
                                                        stem
                                                    } else {
                                                        metadata.id.to_string()
                                                    };
                                                    let rel = format!("dimse/{}/{}.dcm", folder_id, stem);
                                                    let _ = storage.write_file_str(&rel, &bytes).await;
                                                }
                                            }
                                            file_count += 1;
                                            if let Ok(json) =
                                                dicom_json_tool::identifier_to_json_value(object)
                                            {
                                                instances.push(json);
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Skipping C-GET instance that cannot be encoded: {}", e);
                                        }
                                    }
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::warn!("C-GET stream error: {}", e);
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
            "store" => {
                // C-STORE: Send DICOM data to remote PACS
                //
                // Input shapes, in priority order:
                //   1. `normalized_data.dicom_store_parts` — array of base64-encoded
                //      DICOM part payloads, populated by the DICOMweb STOW-RS bridge
                //      (see `DicomwebEndpoint::endpoint_incoming_request`). Each part
                //      results in one C-STORE. Results are aggregated.
                //   2. `envelope.original_data` — one instance. See
                //      `store_dataset_from_body` for the accepted body forms.

                // 1. Multipart-extracted parts path.
                let stow_parts = envelope
                    .normalized_data
                    .as_ref()
                    .and_then(|nd| nd.get("dicom_store_parts"))
                    .and_then(|v| v.as_array())
                    .cloned();

                if let Some(parts) = stow_parts {
                    let total = parts.len();
                    let mut succeeded: usize = 0;
                    let mut failed: usize = 0;
                    let mut errors: Vec<serde_json::Value> = Vec::new();
                    let mut instance_uids: Vec<String> = Vec::new();

                    for (idx, part_val) in parts.iter().enumerate() {
                        let b64 = match part_val.as_str() {
                            Some(s) => s,
                            None => {
                                failed += 1;
                                errors.push(serde_json::json!({
                                    "index": idx,
                                    "error": "part is not a base64 string"
                                }));
                                continue;
                            }
                        };
                        let bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
                            Ok(b) => b,
                            Err(e) => {
                                failed += 1;
                                errors.push(serde_json::json!({
                                    "index": idx,
                                    "error": format!("base64 decode failed: {}", e)
                                }));
                                continue;
                            }
                        };

                        let is_part10 = bytes.len() > 132 && &bytes[128..132] == b"DICM";
                        let (dataset, parsed_obj) = if is_part10 {
                            match dicom_object::from_reader(std::io::Cursor::new(&bytes)) {
                                Ok(file_obj) => {
                                    let inner = file_obj.into_inner();
                                    (DatasetStream::from_object(inner.clone()), Some(inner))
                                }
                                Err(e) => {
                                    failed += 1;
                                    errors.push(serde_json::json!({
                                        "index": idx,
                                        "error": format!("Part 10 parse failed: {}", e)
                                    }));
                                    continue;
                                }
                            }
                        } else {
                            (
                                DatasetStream::from_bytes(bytes::Bytes::from(bytes.clone())),
                                None,
                            )
                        };

                        match scu.store(&remote_node, dataset).await {
                            Ok(true) => {
                                succeeded += 1;
                                if let Some(obj) = parsed_obj {
                                    if let Ok(el) = obj.element_by_name("SOPInstanceUID") {
                                        if let Ok(uid) = el.to_str() {
                                            instance_uids.push(uid.to_string());
                                        }
                                    }
                                }
                            }
                            Ok(false) => {
                                failed += 1;
                                errors.push(serde_json::json!({
                                    "index": idx,
                                    "error": "C-STORE returned false"
                                }));
                            }
                            Err(e) => {
                                failed += 1;
                                errors.push(serde_json::json!({
                                    "index": idx,
                                    "error": e.to_string()
                                }));
                            }
                        }
                    }

                    let mut response = serde_json::json!({
                        "operation": "store",
                        "success": failed == 0 && succeeded > 0,
                        "total_parts": total,
                        "succeeded": succeeded,
                        "failed": failed,
                        "remote_aet": remote_node.ae_title,
                        "host": remote_node.host,
                        "port": remote_node.port,
                    });
                    if !instance_uids.is_empty() {
                        response["sop_instance_uids"] = serde_json::json!(instance_uids);
                    }
                    if !errors.is_empty() {
                        response["errors"] = serde_json::Value::Array(errors);
                    }
                    envelope.normalized_data = Some(response.clone());
                    return Ok(envelope.clone());
                }

                // 2. Single-body path.
                Self::store_single_body(&scu, &remote_node, &envelope.original_data).await
            }
            _ => {
                // This should never be reached due to validation above, but handle it gracefully
                serde_json::json!({
                    "operation": "unknown",
                    "success": false,
                    "error": format!("Unsupported DIMSE operation: '{}'. Valid operations are: echo, find, get, move, store", normalized_op)
                })
            }
        };

        envelope.normalized_data = Some(result);
        Ok(envelope.clone())
    }

    /// Send the one instance in a store request body to `remote_node`.
    ///
    /// Returns the backend's JSON response. `success` is true only when the
    /// remote node took the instance.
    async fn store_single_body(
        scu: &DimseScu,
        remote_node: &RemoteNode,
        body: &[u8],
    ) -> serde_json::Value {
        let dataset = match Self::store_dataset_from_body(body) {
            Ok(dataset) => dataset,
            Err(reason) => {
                return serde_json::json!({
                    "operation": "store",
                    "success": false,
                    "error": reason
                });
            }
        };
        let sop_class_uid = dataset.metadata().sop_class_uid.clone();
        let sop_instance_uid = dataset.metadata().sop_instance_uid.clone();

        match scu.store(remote_node, dataset).await {
            Ok(success) => {
                let mut response = serde_json::json!({
                    "operation": "store",
                    "success": success,
                    "remote_aet": remote_node.ae_title,
                    "host": remote_node.host,
                    "port": remote_node.port
                });
                if let Some(uid) = sop_class_uid {
                    response["sop_class_uid"] = serde_json::json!(uid);
                }
                if let Some(uid) = sop_instance_uid {
                    response["sop_instance_uid"] = serde_json::json!(uid);
                }
                response
            }
            Err(e) => serde_json::json!({
                "operation": "store",
                "success": false,
                "error": e.to_string()
            }),
        }
    }

    /// Read the instance for a C-STORE from a request body.
    ///
    /// Three body forms are accepted:
    /// - A DICOM Part 10 file. An HTTP client sends this.
    /// - A JSON object with the Part 10 file as base64 in `part10_b64`. The
    ///   DIMSE adapter builds this when a `dicom_scp` endpoint is in the same
    ///   pipeline.
    /// - Any other body that does not start with `{`. It is sent as bare data
    ///   set bytes, which the SCU reads as Implicit VR Little Endian.
    ///
    /// A JSON body without `part10_b64` carries no instance and is rejected.
    /// Its `file` key is never read. See `dicom_store_body` for the reason.
    fn store_dataset_from_body(body: &[u8]) -> Result<DatasetStream, String> {
        if body.is_empty() {
            return Err("No DICOM data provided in request body".to_string());
        }
        let part10 = match part10_from_store_body(body) {
            Ok(part10) => part10,
            // The JSON body a `dicom_scp` endpoint builds starts with `{`, and
            // it must be rejected when it cannot be parsed. Data set bytes that
            // start with that byte have no group 0008, so no SOP Class UID,
            // and could not be stored anyway.
            Err(StoreBodyError::NotPart10OrJson { .. }) if body.first() != Some(&b'{') => {
                return Ok(DatasetStream::from_bytes(bytes::Bytes::copy_from_slice(body)));
            }
            Err(e) => return Err(format!("dicom_scu store: {}", e)),
        };
        let file = dicom_object::from_reader(std::io::Cursor::new(part10.as_ref()))
            .map_err(|e| format!("Failed to parse DICOM Part 10 file: {}", e))?;
        Ok(DatasetStream::from_object(file.into_inner()))
    }
}

/// Errors when the envelope still carries the removed `dimse_op` metadata key,
/// in target or request metadata.
///
/// A pipeline that set the operation that way would otherwise run a different
/// operation without anyone noticing: the `x-dimse-op` header from the client,
/// the request path, or the default. `service` names the backend in the error.
pub(crate) fn reject_dimse_op_metadata(
    envelope: &RequestEnvelope<Vec<u8>>,
    service: &str,
) -> Result<(), Error> {
    let in_target = envelope
        .target_details
        .as_ref()
        .is_some_and(|td| td.metadata.contains_key("dimse_op"));
    if !in_target && !envelope.request_details.metadata.contains_key("dimse_op") {
        return Ok(());
    }

    let location = if in_target { "target_details" } else { "request_details" };
    Err(Error::from(format!(
        "{}: {}.metadata.dimse_op is no longer supported. \
         Set the operation with `command` instead (see docs/commands.md).",
        service, location
    )))
}

/// Resolve the DIMSE operation string from a request envelope and backend options.
///
/// Priority (highest first):
/// 1. `envelope.command.operation` — the canonical typed operation
/// 2. `x-dimse-op` HTTP header — set by `dicom_to_http` middleware on the sending side
/// 3. Last non-empty segment of the request URI / `full_path` metadata
///    (catches `/find`, `/get`, `/store` as path-routed endpoints)
/// 4. `dimse_retrieve_mode` backend option, then `"get"` as a last resort
pub(crate) fn resolve_dimse_op(
    envelope: &RequestEnvelope<Vec<u8>>,
    options: &HashMap<String, Value>,
) -> String {
    const VALID_OPS: &[&str] = &["echo", "find", "get", "move", "store"];

    // 1. Typed Command
    // An operation with no DIMSE service falls through to the next step.
    if let Some(op) = envelope.resolve_operation().and_then(|o| o.dimse_op()) {
        return op.to_string();
    }

    // 2. x-dimse-op header (set by dicom_to_http middleware)
    if let Some(op) = envelope
        .request_details
        .headers
        .get("x-dimse-op")
        .map(|s| s.trim_start_matches('/').to_lowercase())
        .filter(|p| VALID_OPS.contains(&p.as_str()))
    {
        return op;
    }

    // 3. Subpath metadata (the portion after the endpoint prefix)
    if let Some(op) = envelope
        .request_details
        .metadata
        .get("path")
        .map(|s| s.trim_start_matches('/').to_lowercase())
        .filter(|p| VALID_OPS.contains(&p.as_str()))
    {
        return op;
    }

    // 3b. Last segment of the full URI path (covers /find, /get, /store as prefixes)
    let full_path = envelope
        .request_details
        .metadata
        .get("full_path")
        .cloned()
        .unwrap_or_else(|| envelope.request_details.uri.clone());

    if let Some(op) = full_path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .last()
        .map(|s| s.to_lowercase())
        .filter(|p| VALID_OPS.contains(&p.as_str()))
    {
        return op;
    }

    // 4. dimse_retrieve_mode option, then "get"
    options
        .get("dimse_retrieve_mode")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "get".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::command::{Command, Operation};
    use crate::models::envelope::envelope::RequestEnvelopeBuilder;

    fn make_envelope(uri: &str) -> RequestEnvelope<Vec<u8>> {
        RequestEnvelopeBuilder::new()
            .method("POST")
            .uri(uri)
            .headers(HashMap::new())
            .cookies(HashMap::new())
            .query_params(HashMap::new())
            .cache_status(None)
            .metadata(HashMap::new())
            .original_data(vec![])
            .build()
            .unwrap()
    }

    fn with_meta(env: &mut RequestEnvelope<Vec<u8>>, key: &str, val: &str) {
        env.request_details.metadata.insert(key.to_string(), val.to_string());
    }

    fn with_header(env: &mut RequestEnvelope<Vec<u8>>, key: &str, val: &str) {
        env.request_details.headers.insert(key.to_string(), val.to_string());
    }

    fn opts() -> HashMap<String, Value> {
        HashMap::new()
    }

    // ── 1. Command takes priority ─────────────────────────────────────────────

    #[test]
    fn test_command_query_maps_to_find() {
        let mut env = make_envelope("/whatever");
        env.command = Some(Command::new(Operation::Query));
        assert_eq!(resolve_dimse_op(&env, &opts()), "find");
    }

    #[test]
    fn test_command_retrieve_maps_to_get() {
        let mut env = make_envelope("/whatever");
        env.command = Some(Command::new(Operation::Retrieve));
        assert_eq!(resolve_dimse_op(&env, &opts()), "get");
    }

    #[test]
    fn test_command_ping_maps_to_echo() {
        let mut env = make_envelope("/whatever");
        env.command = Some(Command::new(Operation::Ping));
        assert_eq!(resolve_dimse_op(&env, &opts()), "echo");
    }

    // ── 2. x-dimse-op header ─────────────────────────────────────────────────

    #[test]
    fn test_header_find() {
        let mut env = make_envelope("/find");
        with_header(&mut env, "x-dimse-op", "find");
        assert_eq!(resolve_dimse_op(&env, &opts()), "find");
    }

    #[test]
    fn test_header_get() {
        let mut env = make_envelope("/get");
        with_header(&mut env, "x-dimse-op", "get");
        assert_eq!(resolve_dimse_op(&env, &opts()), "get");
    }

    #[test]
    fn test_header_invalid_falls_through() {
        let mut env = make_envelope("/find");
        with_header(&mut env, "x-dimse-op", "scan");
        // Falls through to URI last-segment: "find"
        assert_eq!(resolve_dimse_op(&env, &opts()), "find");
    }

    // ── Removed dimse_op metadata key ────────────────────────────────────────

    #[test]
    fn test_reject_dimse_op_accepts_envelope_without_the_key() {
        let mut env = make_envelope("/find");
        with_meta(&mut env, "path", "find");
        assert!(reject_dimse_op_metadata(&env, "dicom_scu").is_ok());
    }

    #[test]
    fn test_reject_dimse_op_in_request_metadata() {
        let mut env = make_envelope("/");
        with_meta(&mut env, "dimse_op", "find");
        let msg = format!("{:?}", reject_dimse_op_metadata(&env, "dicom_scu").unwrap_err());
        assert!(msg.contains("dicom_scu: request_details.metadata.dimse_op"), "{}", msg);
        assert!(msg.contains("command"), "{}", msg);
    }

    #[test]
    fn test_reject_dimse_op_in_target_metadata() {
        let mut env = make_envelope("/");
        env.set_target_metadata("dimse_op", "find");
        let msg = format!("{:?}", reject_dimse_op_metadata(&env, "dicom_scu").unwrap_err());
        assert!(msg.contains("dicom_scu: target_details.metadata.dimse_op"), "{}", msg);
    }

    #[tokio::test]
    async fn test_backend_request_rejects_dimse_op_metadata() {
        // The check must run on the real request path, before any association
        // is opened. 127.0.0.1:9 is never contacted when the check holds.
        let backend = DicomScuBackend {
            local_aet: None,
            aet: Some("REMOTE".to_string()),
            host: Some("127.0.0.1".to_string()),
            port: Some(9),
            use_tls: None,
        };
        let mut env = make_envelope("/find");
        env.set_target_metadata("dimse_op", "find");

        let err = backend
            .handle_backend_request(&mut env, &opts())
            .await
            .expect_err("dimse_op metadata must be rejected");

        let msg = format!("{:?}", err);
        assert!(msg.contains("dicom_scu: target_details.metadata.dimse_op"), "{}", msg);
    }

    // ── 3. Path-based (the bug scenario) ──────────────────────────────────────

    #[test]
    fn test_uri_find_when_subpath_empty() {
        // POST /find with path_prefix="/find" -> subpath=""
        let mut env = make_envelope("/find");
        with_meta(&mut env, "path", "");
        with_meta(&mut env, "full_path", "/find");
        assert_eq!(resolve_dimse_op(&env, &opts()), "find");
    }

    #[test]
    fn test_uri_get_when_subpath_empty() {
        let mut env = make_envelope("/get");
        with_meta(&mut env, "path", "");
        with_meta(&mut env, "full_path", "/get");
        assert_eq!(resolve_dimse_op(&env, &opts()), "get");
    }

    #[test]
    fn test_uri_store_when_subpath_empty() {
        let mut env = make_envelope("/store");
        with_meta(&mut env, "path", "");
        with_meta(&mut env, "full_path", "/store");
        assert_eq!(resolve_dimse_op(&env, &opts()), "store");
    }

    #[test]
    fn test_subpath_find_takes_priority() {
        let mut env = make_envelope("/api/find");
        with_meta(&mut env, "path", "find");
        assert_eq!(resolve_dimse_op(&env, &opts()), "find");
    }

    #[test]
    fn test_query_string_ignored() {
        let mut env = make_envelope("/find?PatientName=Kirk");
        with_meta(&mut env, "path", "");
        with_meta(&mut env, "full_path", "/find?PatientName=Kirk");
        assert_eq!(resolve_dimse_op(&env, &opts()), "find");
    }

    // ── 4. Default fallback ──────────────────────────────────────────────────

    #[test]
    fn test_defaults_to_get() {
        let env = make_envelope("/api/dicom");
        assert_eq!(resolve_dimse_op(&env, &opts()), "get");
    }

    #[test]
    fn test_retrieve_mode_overrides_default() {
        let env = make_envelope("/api/dicom");
        let mut options = opts();
        options.insert("dimse_retrieve_mode".to_string(), Value::String("move".to_string()));
        assert_eq!(resolve_dimse_op(&env, &options), "move");
    }

    // ── C-STORE request bodies ───────────────────────────────────────────────

    fn sample_part10() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/dicom/study_1/series_1/CT.1.1.dcm");
        fs::read(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e))
    }

    fn sample_sop_instance_uid() -> String {
        let file = dicom_object::from_reader(std::io::Cursor::new(sample_part10()))
            .expect("sample instance parses");
        let uid = file
            .element_by_name("SOPInstanceUID")
            .expect("sample has a SOP Instance UID")
            .to_str()
            .expect("UID is text")
            .to_string();
        uid
    }

    fn json_body(body: Value) -> Vec<u8> {
        serde_json::to_vec(&body).expect("serialise body")
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn store_body_error(body: &[u8]) -> String {
        match DicomScuBackend::store_dataset_from_body(body) {
            Ok(_) => panic!("the body should be rejected"),
            Err(reason) => reason,
        }
    }

    #[test]
    fn test_store_dataset_reads_part10_b64_and_ignores_file() {
        // AURA-2476. This is the body a `dicom_scp` endpoint builds. The path
        // does not exist, so reading it would fail the request.
        let body = json_body(serde_json::json!({
            "command": {"message_id": 1},
            "dataset": {},
            "file": "/nonexistent/aura-2476/instance.dcm",
            "part10_b64": b64(&sample_part10()),
        }));

        let dataset = DicomScuBackend::store_dataset_from_body(&body).expect("instance");

        assert!(matches!(dataset, DatasetStream::Object { .. }));
        assert_eq!(
            dataset.metadata().sop_instance_uid,
            Some(sample_sop_instance_uid())
        );
    }

    #[test]
    fn test_store_dataset_never_reads_a_file_named_by_the_request() {
        // The file is a valid instance, so the only way this request can fail
        // is that the backend did not open it.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("instance.dcm");
        fs::write(&file, sample_part10()).expect("write instance");

        let reason = store_body_error(&json_body(serde_json::json!({
            "command": {"message_id": 1},
            "dataset": {},
            "file": file.to_string_lossy(),
        })));

        assert!(reason.contains("does not read files"), "unexpected error: {}", reason);
    }

    #[test]
    fn test_store_dataset_accepts_part10_body() {
        let dataset =
            DicomScuBackend::store_dataset_from_body(&sample_part10()).expect("instance");

        assert!(matches!(dataset, DatasetStream::Object { .. }));
        assert_eq!(
            dataset.metadata().sop_instance_uid,
            Some(sample_sop_instance_uid())
        );
    }

    #[test]
    fn test_store_dataset_sends_other_bytes_as_a_bare_data_set() {
        // (0008,0016) SOP Class UID, Implicit VR Little Endian. Not Part 10
        // and not JSON.
        let bare = [0x08, 0x00, 0x16, 0x00, 0x02, 0x00, 0x00, 0x00, b'1', 0x00];

        let dataset = DicomScuBackend::store_dataset_from_body(&bare).expect("data set");

        match &dataset {
            DatasetStream::Memory { data, .. } => assert_eq!(data.as_ref(), &bare[..]),
            other => panic!("expected the body as bare bytes, got {:?}", other.metadata()),
        }
    }

    #[test]
    fn test_store_dataset_never_sends_unreadable_json_as_a_data_set() {
        // serde_json writes a data set with many nested sequences but refuses
        // to read it back. That body holds a local path and must not go to
        // the PACS as data set bytes.
        let nested = format!("{{\"dataset\":{}{}}}", "[".repeat(200), "]".repeat(200));

        let reason = store_body_error(nested.as_bytes());

        assert!(
            reason.contains("neither a DICOM Part 10 file nor JSON"),
            "unexpected error: {}",
            reason
        );
    }

    #[test]
    fn test_store_dataset_rejects_bodies_without_an_instance() {
        let rejected = [
            (Vec::new(), "No DICOM data provided"),
            (json_body(serde_json::json!({"dataset": {}})), "no 'part10_b64' string"),
            (json_body(serde_json::json!({"part10_b64": 7})), "no 'part10_b64' string"),
            (
                json_body(serde_json::json!({"part10_b64": "not base64!"})),
                "not valid base64",
            ),
            (
                json_body(serde_json::json!({"part10_b64": b64(b"bare data set bytes")})),
                "not a DICOM Part 10 file",
            ),
        ];
        for (body, expected) in rejected {
            let reason = store_body_error(&body);
            assert!(reason.contains(expected), "expected {:?} in: {}", expected, reason);
        }
    }

    #[test]
    fn test_store_dataset_rejects_part10_magic_without_a_data_set() {
        let mut truncated = vec![0u8; 128];
        truncated.extend_from_slice(b"DICM");
        truncated.extend_from_slice(&[0u8; 8]);

        let reason = store_body_error(&truncated);

        assert!(
            reason.contains("Failed to parse DICOM Part 10 file"),
            "unexpected error: {}",
            reason
        );
    }
}
