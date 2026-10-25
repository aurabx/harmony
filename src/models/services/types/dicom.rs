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
use crate::models::middleware::types::dicomweb_to_dicom::{
    StowPartResult, STOW_FAILURE_CANNOT_UNDERSTAND, STOW_FAILURE_PROCESSING,
};
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
            .map_err(|e| Error::from(format!("dicom_scu backend request failed: {}", e)))?;

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
                    Ok(stream) => match collect_find_matches(stream).await {
                        Ok(matches) => serde_json::json!({
                            "operation": "find",
                            "success": true,
                            "matches": matches
                        }),
                        Err(e) => dimse_failure_response("find", &e),
                    },
                    Err(e) => dimse_failure_response("find", &e),
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
                            let any = match move_preflight_outcome(stream.next().await) {
                                MovePreflight::Proceed => true,
                                MovePreflight::StudyNotFound => false,
                                MovePreflight::Unreachable(e) => {
                                    envelope.normalized_data =
                                        Some(dimse_failure_response("move", &e));
                                    return Ok(envelope.clone());
                                }
                            };
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
                            let item = match item {
                                Ok(item) => item,
                                Err(e) => {
                                    envelope.normalized_data =
                                        Some(dimse_failure_response("move", &e));
                                    return Ok(envelope.clone());
                                }
                            };
                            if let dimse::types::DatasetStream::File { ref path, .. } = item {
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
                                            match item2 {
                                                Ok(dimse::types::DatasetStream::File {
                                                    ref path,
                                                    ..
                                                }) => {
                                                    if path.is_file() {
                                                        produced += 1;
                                                    }
                                                }
                                                Ok(_) => {}
                                                Err(e) => {
                                                    envelope.normalized_data =
                                                        Some(dimse_failure_response("move", &e));
                                                    return Ok(envelope.clone());
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
                    Err(e) => dimse_failure_response("move", &e),
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
                                Ok(
                                    ref dataset @ dimse::types::DatasetStream::Object {
                                        ref object,
                                        ref metadata,
                                    },
                                ) => {
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
                                                    let stem =
                                                        if stem.chars().any(|c| c.is_ascii_digit())
                                                        {
                                                            stem
                                                        } else {
                                                            metadata.id.to_string()
                                                        };
                                                    let rel =
                                                        format!("dimse/{}/{}.dcm", folder_id, stem);
                                                    let _ =
                                                        storage.write_file_str(&rel, &bytes).await;
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
                                    // A transport or association failure. The SCU ends
                                    // the stream after it, so any instances so far are
                                    // incomplete and the retrieve has failed.
                                    envelope.normalized_data =
                                        Some(dimse_failure_response("get", &e));
                                    return Ok(envelope.clone());
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
                    Err(e) => dimse_failure_response("get", &e),
                }
            }
            "store" => {
                // C-STORE: Send DICOM data to remote PACS
                //
                // Input shapes, in priority order:
                //   1. `normalized_data.dicom_store_parts` — array of base64-encoded
                //      DICOM part payloads, populated by the DICOMweb STOW-RS bridge
                //      (see `DicomwebEndpoint::endpoint_incoming_request`). Each part
                //      results in one C-STORE. See `store_stow_parts`.
                //   2. `envelope.original_data` — one instance. See
                //      `store_dataset_from_body` for the accepted body forms.

                // Borrowed, not cloned: the parts hold every instance as base64.
                let stow_parts = envelope
                    .normalized_data
                    .as_ref()
                    .and_then(|nd| nd.get("dicom_store_parts"))
                    .and_then(|v| v.as_array());

                match stow_parts {
                    Some(parts) => Self::store_stow_parts(&scu, &remote_node, parts).await,
                    None => {
                        Self::store_single_body(&scu, &remote_node, &envelope.original_data).await
                    }
                }
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

    /// Send each STOW-RS part to `remote_node`, one C-STORE per part.
    ///
    /// `parts` is `normalized_data.dicom_store_parts`: base64 strings, one per
    /// `application/dicom` part. Each part is decoded only when it is sent.
    /// Returns the backend's JSON response. Its `results` array holds one
    /// `StowPartResult` per part, in order. The `dicomweb_to_dicom` middleware
    /// turns that array into the PS3.18 Store Instances Response.
    async fn store_stow_parts(
        scu: &DimseScu,
        remote_node: &RemoteNode,
        parts: &[serde_json::Value],
    ) -> serde_json::Value {
        let mut results: Vec<StowPartResult> = Vec::with_capacity(parts.len());
        for (index, part) in parts.iter().enumerate() {
            results.push(Self::store_stow_part(scu, remote_node, index, part).await);
        }

        let succeeded = results.iter().filter(|r| r.success).count();
        let failed = results.len() - succeeded;
        let errors: Vec<serde_json::Value> = results
            .iter()
            .filter_map(|r| {
                r.error
                    .as_ref()
                    .map(|e| serde_json::json!({"index": r.index, "error": e}))
            })
            .collect();
        let instance_uids: Vec<&str> = results
            .iter()
            .filter(|r| r.success)
            .filter_map(|r| r.sop_instance_uid.as_deref())
            .collect();

        let mut response = serde_json::json!({
            "operation": "store",
            "success": failed == 0 && succeeded > 0,
            "total_parts": parts.len(),
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
        response["results"] = results.iter().map(StowPartResult::to_json).collect();
        response
    }

    /// Send one STOW-RS part to `remote_node` and return its result.
    ///
    /// A part that `decode_stow_part` rejects is not sent. It fails with
    /// Failure Reason 0xC000. A part the remote node does not take fails with
    /// 0x0110.
    async fn store_stow_part(
        scu: &DimseScu,
        remote_node: &RemoteNode,
        index: usize,
        part: &serde_json::Value,
    ) -> StowPartResult {
        let instance = match decode_stow_part(part) {
            Ok(instance) => instance,
            Err(reason) => {
                return stow_part_result(
                    index,
                    None,
                    None,
                    Some((STOW_FAILURE_CANNOT_UNDERSTAND, reason)),
                );
            }
        };
        let failure = match scu.store(remote_node, instance.dataset).await {
            Ok(true) => None,
            Ok(false) => Some("C-STORE returned false".to_string()),
            Err(e) => Some(e.to_string()),
        };
        stow_part_result(
            index,
            Some(instance.sop_class_uid),
            Some(instance.sop_instance_uid),
            failure.map(|reason| (STOW_FAILURE_PROCESSING, reason)),
        )
    }

    /// Send the one instance in a store request body to `remote_node`.
    ///
    /// Returns the backend's JSON response. `success` is true only when the
    /// remote node took the instance. `results` holds one `StowPartResult`, so
    /// a DICOMweb client gets a PS3.18 response for a single-body STOW-RS
    /// request too.
    async fn store_single_body(
        scu: &DimseScu,
        remote_node: &RemoteNode,
        body: &[u8],
    ) -> serde_json::Value {
        let dataset = match Self::store_dataset_from_body(body) {
            Ok(dataset) => dataset,
            Err(reason) => {
                let result = stow_part_result(
                    0,
                    None,
                    None,
                    Some((STOW_FAILURE_CANNOT_UNDERSTAND, reason.clone())),
                );
                return serde_json::json!({
                    "operation": "store",
                    "success": false,
                    "error": reason,
                    "results": [result.to_json()]
                });
            }
        };
        let (sop_class_uid, sop_instance_uid) = sop_uids_of(&dataset);

        let outcome = scu.store(remote_node, dataset).await;
        let failure = match &outcome {
            Ok(true) => None,
            Ok(false) => Some("C-STORE returned false".to_string()),
            Err(e) => Some(e.to_string()),
        };
        let result = stow_part_result(
            0,
            sop_class_uid.clone(),
            sop_instance_uid.clone(),
            failure.map(|reason| (STOW_FAILURE_PROCESSING, reason)),
        );

        match outcome {
            Ok(success) => {
                let mut response = serde_json::json!({
                    "operation": "store",
                    "success": success,
                    "remote_aet": remote_node.ae_title,
                    "host": remote_node.host,
                    "port": remote_node.port,
                    "results": [result.to_json()]
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
                "error": e.to_string(),
                "results": [result.to_json()]
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
                return Ok(DatasetStream::from_bytes(bytes::Bytes::copy_from_slice(
                    body,
                )));
            }
            Err(e) => return Err(format!("dicom_scu store: {}", e)),
        };
        let file = dicom_object::from_reader(std::io::Cursor::new(part10.as_ref()))
            .map_err(|e| format!("Failed to parse DICOM Part 10 file: {}", e))?;
        // The SCU proposes this transfer syntax to the PACS. Encapsulated
        // pixel data can only be sent in the transfer syntax it was encoded in.
        let transfer_syntax = file.meta().transfer_syntax().to_string();
        let mut dataset = DatasetStream::from_object(file.into_inner());
        dataset.metadata_mut().transfer_syntax = Some(transfer_syntax);
        Ok(dataset)
    }
}

/// A STOW-RS part that is ready for C-STORE.
struct StowInstance {
    dataset: DatasetStream,
    sop_class_uid: String,
    sop_instance_uid: String,
}

/// Decode one `dicom_store_parts` entry into an instance to send.
///
/// Only a base64 DICOM Part 10 file that carries a SOP Class UID and a SOP
/// Instance UID is accepted. PS3.18 sends STOW-RS instances as Part 10 files.
/// The UIDs are read before sending so the response can name the instance
/// whether or not the store succeeds. The error says why the part was refused.
fn decode_stow_part(part: &Value) -> Result<StowInstance, String> {
    let b64 = part
        .as_str()
        .ok_or_else(|| "part is not a base64 string".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode failed: {}", e))?;
    if !(bytes.len() > 132 && &bytes[128..132] == b"DICM") {
        return Err("part is not a DICOM Part 10 file".to_string());
    }
    let file = dicom_object::from_reader(std::io::Cursor::new(&bytes))
        .map_err(|e| format!("Part 10 parse failed: {}", e))?;
    // The SCU proposes this transfer syntax to the PACS. Encapsulated pixel
    // data can only be sent in the transfer syntax it was encoded in.
    let transfer_syntax = file.meta().transfer_syntax().to_string();
    let mut dataset = DatasetStream::from_object(file.into_inner());
    dataset.metadata_mut().transfer_syntax = Some(transfer_syntax);

    let non_empty = |uid: &Option<String>| uid.clone().filter(|u| !u.trim().is_empty());
    let sop_class_uid = non_empty(&dataset.metadata().sop_class_uid)
        .ok_or_else(|| "part has no SOP Class UID (0008,0016)".to_string())?;
    let sop_instance_uid = non_empty(&dataset.metadata().sop_instance_uid)
        .ok_or_else(|| "part has no SOP Instance UID (0008,0018)".to_string())?;
    Ok(StowInstance {
        dataset,
        sop_class_uid,
        sop_instance_uid,
    })
}

/// Build the `StowPartResult` for one instance.
///
/// `failure` is the PS3.18 Failure Reason and a message. `None` means the
/// remote node took the instance. UIDs that are not known are `None`.
fn stow_part_result(
    index: usize,
    sop_class_uid: Option<String>,
    sop_instance_uid: Option<String>,
    failure: Option<(u16, String)>,
) -> StowPartResult {
    let (failure_reason, error) = match failure {
        Some((reason, message)) => (Some(reason), Some(message)),
        None => (None, None),
    };
    StowPartResult {
        index,
        success: failure_reason.is_none(),
        sop_class_uid,
        sop_instance_uid,
        failure_reason,
        error,
    }
}

/// The SOP Class UID and SOP Instance UID of an instance about to be sent.
///
/// A Part 10 instance carries them in its metadata. Bare data set bytes do
/// not, so they are read the way the SCU reads those bytes: as Implicit VR
/// Little Endian. The SCU refuses to send an instance without both UIDs, so a
/// successful store means both are readable there.
fn sop_uids_of(dataset: &DatasetStream) -> (Option<String>, Option<String>) {
    let metadata = dataset.metadata();
    let (class_uid, instance_uid) = match dataset {
        DatasetStream::Memory { data, .. }
            if metadata.sop_class_uid.is_none() || metadata.sop_instance_uid.is_none() =>
        {
            implicit_vr_sop_uids(data)
        }
        _ => (None, None),
    };
    (
        metadata.sop_class_uid.clone().or(class_uid),
        metadata.sop_instance_uid.clone().or(instance_uid),
    )
}

/// Best-effort read of SOP Class UID (0008,0016) and SOP Instance UID
/// (0008,0018) from bare Implicit VR Little Endian data set bytes.
///
/// Elements are in tag order, so the walk stops after (0008,0018). It also
/// stops at an undefined length, a truncated element, or a value that is not
/// a UID. A UID it cannot read is `None`. This only feeds the STOW-RS response
/// and never decides whether an instance is sent.
fn implicit_vr_sop_uids(data: &[u8]) -> (Option<String>, Option<String>) {
    const SOP_CLASS_UID: (u16, u16) = (0x0008, 0x0016);
    const SOP_INSTANCE_UID: (u16, u16) = (0x0008, 0x0018);
    let mut class_uid = None;
    let mut instance_uid = None;
    let mut pos: usize = 0;
    while let Some(header) = data.get(pos..pos + 8) {
        let tag = (
            u16::from_le_bytes([header[0], header[1]]),
            u16::from_le_bytes([header[2], header[3]]),
        );
        let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if tag > SOP_INSTANCE_UID || length == u32::MAX {
            break;
        }
        let start = pos + 8;
        // An end past `usize` is treated like a truncated element.
        let Some(end) = start.checked_add(length as usize) else {
            break;
        };
        let Some(value) = data.get(start..end) else {
            break;
        };
        if tag == SOP_CLASS_UID {
            class_uid = uid_from_value(value);
        } else if tag == SOP_INSTANCE_UID {
            instance_uid = uid_from_value(value);
        }
        pos = end;
    }
    (class_uid, instance_uid)
}

/// The longest value a UI element may hold, in bytes (PS3.5 Table 6.2-1).
const MAX_UID_LEN: usize = 64;

/// A UI value without its padding, or `None` if it is not a UID.
fn uid_from_value(value: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(value).ok()?;
    let uid = text.trim_end_matches(['\0', ' ']);
    let is_uid = !uid.is_empty()
        && uid.len() <= MAX_UID_LEN
        && uid.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    is_uid.then(|| uid.to_string())
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

    let location = if in_target {
        "target_details"
    } else {
        "request_details"
    };
    Err(Error::from(format!(
        "{}: {}.metadata.dimse_op is no longer supported. \
         Set the operation with `command` instead (see docs/architecture/commands.md).",
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

/// The backend response for a failed C-FIND, C-GET or C-MOVE.
///
/// Any endpoint may send this JSON to its client, so `error` holds a fixed
/// message. The raw error names hosts, ports and AE titles. It is logged here
/// and goes no further. `operation` and `success: false` stay so middleware
/// such as `dicomweb_bridge` can detect the failure.
fn dimse_failure_response(operation: &str, error: &dyn std::fmt::Display) -> Value {
    let message = match operation {
        "find" => "C-FIND failed",
        "get" => "C-GET failed",
        "move" => "C-MOVE failed",
        _ => "DIMSE operation failed",
    };
    warn!(
        operation = operation,
        "DIMSE {} failed: {}", operation, error
    );
    serde_json::json!({
        "operation": operation,
        "success": false,
        "error": message
    })
}

/// Collects C-FIND results as DICOM JSON identifiers.
///
/// Returns the first stream error. The SCU ends the stream after an error,
/// so the matches before it are incomplete and must not be reported as a
/// complete answer. Results that cannot be decoded are skipped, as before.
async fn collect_find_matches<S>(mut stream: S) -> dimse::Result<Vec<Value>>
where
    S: futures_util::Stream<Item = dimse::Result<DatasetStream>> + Unpin,
{
    use futures_util::StreamExt;
    let mut matches: Vec<Value> = Vec::new();
    while let Some(item) = stream.next().await {
        let json = match item? {
            DatasetStream::File { ref path, .. } => dicom_object::open_file(path)
                .ok()
                .and_then(|obj| dicom_json_tool::identifier_to_json_value(&obj).ok()),
            DatasetStream::Memory { ref data, .. } => dicom_object::from_reader(&**data)
                .ok()
                .and_then(|obj| dicom_json_tool::identifier_to_json_value(&obj).ok()),
            DatasetStream::Object { ref object, .. } => {
                dicom_json_tool::identifier_to_json_value(object).ok()
            }
        };
        if let Some(json) = json {
            matches.push(json);
        }
    }
    Ok(matches)
}

/// What the C-FIND preflight before a C-MOVE allows.
#[derive(Debug)]
enum MovePreflight {
    /// Go ahead with the C-MOVE.
    Proceed,
    /// The C-FIND answered with no match for the study.
    StudyNotFound,
    /// The node cannot be reached, so the C-MOVE would fail too.
    Unreachable(dimse::DimseError),
}

/// Decides the C-MOVE preflight from the first C-FIND stream item.
///
/// Only a connection or association failure stops the move. A PACS may
/// support C-MOVE but refuse C-FIND, or answer C-FIND with a failure status.
/// The move still has a chance there, so it goes ahead as it did before the
/// preflight could see errors.
fn move_preflight_outcome(first: Option<dimse::Result<DatasetStream>>) -> MovePreflight {
    match first {
        Some(Ok(_)) => MovePreflight::Proceed,
        None => MovePreflight::StudyNotFound,
        Some(Err(e)) if e.is_connection_failure() => MovePreflight::Unreachable(e),
        Some(Err(e)) => {
            warn!(
                "C-MOVE preflight C-FIND failed, trying the move anyway: {}",
                e
            );
            MovePreflight::Proceed
        }
    }
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
        env.request_details
            .metadata
            .insert(key.to_string(), val.to_string());
    }

    fn with_header(env: &mut RequestEnvelope<Vec<u8>>, key: &str, val: &str) {
        env.request_details
            .headers
            .insert(key.to_string(), val.to_string());
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
        let msg = format!(
            "{:?}",
            reject_dimse_op_metadata(&env, "dicom_scu").unwrap_err()
        );
        assert!(
            msg.contains("dicom_scu: request_details.metadata.dimse_op"),
            "{}",
            msg
        );
        assert!(msg.contains("command"), "{}", msg);
    }

    #[test]
    fn test_reject_dimse_op_in_target_metadata() {
        let mut env = make_envelope("/");
        env.set_target_metadata("dimse_op", "find");
        let msg = format!(
            "{:?}",
            reject_dimse_op_metadata(&env, "dicom_scu").unwrap_err()
        );
        assert!(
            msg.contains("dicom_scu: target_details.metadata.dimse_op"),
            "{}",
            msg
        );
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
        assert!(
            msg.contains("dicom_scu: target_details.metadata.dimse_op"),
            "{}",
            msg
        );
    }

    #[tokio::test]
    async fn test_backend_outgoing_request_returns_error_when_handler_fails() {
        // A failing handle_backend_request must surface as an Err, not a panic.
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
            .backend_outgoing_request(env, &opts())
            .await
            .expect_err("a handler failure must be returned as an error");

        let msg = err.to_string();
        assert!(
            msg.starts_with("dicom_scu backend request failed: "),
            "{}",
            msg
        );
        assert!(
            msg.contains("dicom_scu: target_details.metadata.dimse_op"),
            "{}",
            msg
        );
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
        options.insert(
            "dimse_retrieve_mode".to_string(),
            Value::String("move".to_string()),
        );
        assert_eq!(resolve_dimse_op(&env, &options), "move");
    }

    // ── C-STORE request bodies ───────────────────────────────────────────────

    fn sample_part10() -> Vec<u8> {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/dicom/study_1/series_1/CT.1.1.dcm");
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

        assert!(
            reason.contains("does not read files"),
            "unexpected error: {}",
            reason
        );
    }

    #[test]
    fn test_store_dataset_accepts_part10_body() {
        let dataset = DicomScuBackend::store_dataset_from_body(&sample_part10()).expect("instance");

        assert!(matches!(dataset, DatasetStream::Object { .. }));
        assert_eq!(
            dataset.metadata().sop_instance_uid,
            Some(sample_sop_instance_uid())
        );
    }

    #[test]
    fn test_store_dataset_records_the_transfer_syntax_of_the_file() {
        let file = dicom_object::from_reader(std::io::Cursor::new(sample_part10()))
            .expect("sample instance parses");
        let expected = file.meta().transfer_syntax().to_string();

        let dataset = DicomScuBackend::store_dataset_from_body(&sample_part10()).expect("instance");

        assert_eq!(dataset.metadata().transfer_syntax, Some(expected));
    }

    // ── C-STORE of a STOW-RS part ────────────────────────────────────────────

    /// What the test PACS was handed for one C-STORE: the negotiated transfer
    /// syntax and the instance.
    type ReceivedParts =
        std::sync::Arc<std::sync::Mutex<Vec<(Option<String>, dicom_object::InMemDicomObject)>>>;

    struct RecordingPacs {
        received: ReceivedParts,
    }

    #[async_trait]
    impl dimse::scp::QueryProvider for RecordingPacs {
        async fn find(
            &self,
            _peer_addr: std::net::SocketAddr,
            _query_level: QueryLevel,
            _parameters: &HashMap<String, String>,
            _max_results: u32,
        ) -> dimse::Result<dimse::types::BoxDatasetStream> {
            Ok(dimse::types::dataset_stream_from_vec(Vec::new()))
        }

        async fn locate(
            &self,
            _peer_addr: std::net::SocketAddr,
            _query_level: QueryLevel,
            _parameters: &HashMap<String, String>,
        ) -> dimse::Result<dimse::types::BoxDatasetStream> {
            Ok(dimse::types::dataset_stream_from_vec(Vec::new()))
        }

        async fn get(
            &self,
            _peer_addr: std::net::SocketAddr,
            _query_level: QueryLevel,
            _parameters: &HashMap<String, String>,
        ) -> dimse::Result<dimse::types::RetrieveStream> {
            Ok(dimse::types::RetrieveStream::new(
                dimse::types::dataset_stream_from_vec(Vec::new()),
            ))
        }

        async fn store(
            &self,
            _peer_addr: std::net::SocketAddr,
            dataset: DatasetStream,
        ) -> dimse::Result<()> {
            let transfer_syntax = dataset.metadata().transfer_syntax.clone();
            let object = dataset.to_object().await?;
            self.received
                .lock()
                .unwrap()
                .push((transfer_syntax, object));
            Ok(())
        }
    }

    /// Start a PACS on a free local port that accepts every transfer syntax.
    async fn start_recording_pacs() -> (u16, ReceivedParts, tempfile::TempDir) {
        let received: ReceivedParts = Default::default();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind a free port")
            .local_addr()
            .expect("local address")
            .port();
        let storage_dir = tempfile::tempdir().expect("PACS storage dir");
        let scp = dimse::DimseScp::new(
            DimseConfig {
                local_aet: "STOW_PACS".to_string(),
                bind_addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                port,
                enable_store: true,
                storage_dir: storage_dir.path().to_path_buf(),
                ..Default::default()
            },
            std::sync::Arc::new(RecordingPacs {
                received: received.clone(),
            }),
        );
        tokio::spawn(scp.run(tokio_util::sync::CancellationToken::new()));
        for _ in 0..250 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return (port, received, storage_dir);
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the PACS did not start listening on port {}", port);
    }

    #[tokio::test]
    async fn test_stow_part_keeps_its_compressed_transfer_syntax() {
        use dicom_core::value::{PixelFragmentSequence, Value as DicomValue};
        use dicom_core::{DataElement, PrimitiveValue, VR};
        use dicom_dictionary_std::{tags, uids};

        const JPEG_LOSSLESS_SV1: &str = "1.2.840.10008.1.2.4.70";
        // Not a real JPEG stream. Nothing on the path decodes the fragment.
        let fragments = vec![vec![0xFFu8, 0xD8, 0x01, 0x02, 0x03, 0x04, 0xFF, 0xD9]];

        let mut object = dicom_object::InMemDicomObject::new_empty();
        for (tag, value) in [
            (tags::SOP_CLASS_UID, uids::SECONDARY_CAPTURE_IMAGE_STORAGE),
            (tags::SOP_INSTANCE_UID, "1.2.826.0.1.3680043.9.7133.2470.8"),
        ] {
            object.put(DataElement::new(tag, VR::UI, PrimitiveValue::from(value)));
        }
        object.put(DataElement::new(
            tags::PIXEL_DATA,
            VR::OB,
            DicomValue::PixelSequence(PixelFragmentSequence::new_fragments(fragments.clone())),
        ));
        let mut part10 = Vec::new();
        object
            .with_meta(
                dicom_object::meta::FileMetaTableBuilder::new().transfer_syntax(JPEG_LOSSLESS_SV1),
            )
            .expect("file meta group")
            .write_all(&mut part10)
            .expect("encode the Part 10 file");

        let (port, received, _storage_dir) = start_recording_pacs().await;
        let backend = DicomScuBackend {
            local_aet: Some("STOW_SCU".to_string()),
            aet: Some("STOW_PACS".to_string()),
            host: Some("127.0.0.1".to_string()),
            port: Some(port),
            use_tls: None,
        };
        // The `dicomweb` endpoint hands each STOW-RS part over this way.
        let mut env = make_envelope("/store");
        env.normalized_data = Some(serde_json::json!({
            "dicom_store_parts": [base64::engine::general_purpose::STANDARD.encode(&part10)]
        }));

        let response = backend
            .handle_backend_request(&mut env, &opts())
            .await
            .expect("the backend answers a store");

        let result = response.normalized_data.expect("store result");
        assert_eq!(result["success"], true, "store result: {}", result);
        // The DICOMweb bridge builds the PS3.18 response from these entries.
        assert_eq!(
            result["results"],
            serde_json::json!([{
                "index": 0,
                "success": true,
                "sop_class_uid": uids::SECONDARY_CAPTURE_IMAGE_STORAGE,
                "sop_instance_uid": "1.2.826.0.1.3680043.9.7133.2470.8"
            }]),
            "store result: {}",
            result
        );

        let received = received.lock().unwrap();
        assert_eq!(
            received.len(),
            1,
            "the PACS should hold exactly one instance"
        );
        assert_eq!(received[0].0.as_deref(), Some(JPEG_LOSSLESS_SV1));
        match received[0]
            .1
            .element(tags::PIXEL_DATA)
            .expect("PixelData missing")
            .value()
        {
            DicomValue::PixelSequence(sequence) => {
                assert_eq!(sequence.fragments(), fragments.as_slice())
            }
            other => panic!("PixelData is no longer encapsulated: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_stow_part_that_is_not_part10_is_never_sent() {
        let (port, received, _storage_dir) = start_recording_pacs().await;
        let backend = DicomScuBackend {
            local_aet: Some("STOW_SCU".to_string()),
            aet: Some("STOW_PACS".to_string()),
            host: Some("127.0.0.1".to_string()),
            port: Some(port),
            use_tls: None,
        };
        // (0008,0016) SOP Class UID as bare Implicit VR Little Endian bytes.
        let bare = [0x08, 0x00, 0x16, 0x00, 0x02, 0x00, 0x00, 0x00, b'1', 0x00];
        let mut env = make_envelope("/store");
        env.normalized_data = Some(serde_json::json!({
            "dicom_store_parts": [b64(&bare)]
        }));

        let response = backend
            .handle_backend_request(&mut env, &opts())
            .await
            .expect("the backend answers a store");

        let result = response.normalized_data.expect("store result");
        assert_eq!(result["success"], false, "store result: {}", result);
        let entry = &result["results"][0];
        assert_eq!(entry["success"], false, "entry: {}", entry);
        assert_eq!(entry["failure_reason"], 49152, "entry: {}", entry);
        assert!(entry.get("sop_class_uid").is_none(), "entry: {}", entry);
        assert!(entry.get("sop_instance_uid").is_none(), "entry: {}", entry);
        assert!(
            received.lock().unwrap().is_empty(),
            "a part that is not a Part 10 file must not reach the PACS"
        );
    }

    #[tokio::test]
    async fn test_stow_part_the_pacs_does_not_take_keeps_its_uids() {
        // Nothing listens on this port, so the C-STORE cannot connect.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind a free port")
            .local_addr()
            .expect("local address")
            .port();
        let backend = DicomScuBackend {
            local_aet: Some("STOW_SCU".to_string()),
            aet: Some("STOW_PACS".to_string()),
            host: Some("127.0.0.1".to_string()),
            port: Some(port),
            use_tls: None,
        };
        let mut env = make_envelope("/store");
        env.normalized_data = Some(serde_json::json!({
            "dicom_store_parts": [b64(&sample_part10())]
        }));

        let response = backend
            .handle_backend_request(&mut env, &opts())
            .await
            .expect("the backend answers a store");

        let result = response.normalized_data.expect("store result");
        let entry = &result["results"][0];
        assert_eq!(entry["success"], false, "entry: {}", entry);
        assert_eq!(entry["failure_reason"], 272, "entry: {}", entry);
        assert_eq!(
            entry["sop_instance_uid"],
            sample_sop_instance_uid(),
            "entry: {}",
            entry
        );
        assert!(entry["sop_class_uid"].is_string(), "entry: {}", entry);
        assert!(entry["error"].is_string(), "entry: {}", entry);
    }

    /// One Implicit VR Little Endian element.
    fn implicit_element(group: u16, element: u16, value: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&group.to_le_bytes());
        bytes.extend_from_slice(&element.to_le_bytes());
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    #[test]
    fn test_sop_uids_of_reads_bare_data_set_bytes() {
        let mut bare = implicit_element(0x0008, 0x0005, b"ISO_IR 100");
        bare.extend(implicit_element(
            0x0008,
            0x0016,
            b"1.2.840.10008.5.1.4.1.1.7\0",
        ));
        bare.extend(implicit_element(0x0008, 0x0018, b"1.2.3.4 "));
        bare.extend(implicit_element(0x0010, 0x0010, b"Doe^John"));
        let dataset = DicomScuBackend::store_dataset_from_body(&bare).expect("data set");

        assert_eq!(
            sop_uids_of(&dataset),
            (
                Some("1.2.840.10008.5.1.4.1.1.7".to_string()),
                Some("1.2.3.4".to_string())
            )
        );
    }

    #[test]
    fn test_uid_longer_than_64_bytes_is_rejected() {
        let longest = format!("1.{}", "2".repeat(62));
        assert_eq!(longest.len(), 64);
        let too_long = format!("{}3", longest);

        // Padding does not count towards the limit.
        let padded = format!("{}\0", longest);
        assert_eq!(uid_from_value(padded.as_bytes()), Some(longest.clone()));
        assert_eq!(uid_from_value(too_long.as_bytes()), None);

        let mut bare = implicit_element(0x0008, 0x0016, longest.as_bytes());
        bare.extend(implicit_element(
            0x0008,
            0x0018,
            format!("{} ", too_long).as_bytes(),
        ));
        assert_eq!(implicit_vr_sop_uids(&bare), (Some(longest), None));
    }

    #[test]
    fn test_implicit_vr_sop_uids_gives_up_safely() {
        // Truncated: the length runs past the end of the bytes.
        let mut truncated = implicit_element(0x0008, 0x0016, b"1.2.3");
        truncated.truncate(truncated.len() - 2);
        assert_eq!(implicit_vr_sop_uids(&truncated), (None, None));

        // Undefined length before the UIDs.
        let mut undefined = vec![0x08, 0x00, 0x06, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        undefined.extend(implicit_element(0x0008, 0x0016, b"1.2.3"));
        assert_eq!(implicit_vr_sop_uids(&undefined), (None, None));

        // A value that is not a UID.
        let not_uid = implicit_element(0x0008, 0x0018, b"Doe^John");
        assert_eq!(implicit_vr_sop_uids(&not_uid), (None, None));

        // A length whose end is past `usize::MAX` on 32-bit targets, and past
        // the data everywhere.
        let mut huge = vec![0x08, 0x00, 0x16, 0x00, 0xFE, 0xFF, 0xFF, 0xFF];
        huge.extend_from_slice(b"1.2.3");
        assert_eq!(implicit_vr_sop_uids(&huge), (None, None));

        assert_eq!(implicit_vr_sop_uids(&[]), (None, None));
    }

    #[test]
    fn test_decode_stow_part_rejects_unusable_parts() {
        let mut no_uids = Vec::new();
        dicom_object::InMemDicomObject::new_empty()
            .with_meta(
                dicom_object::meta::FileMetaTableBuilder::new()
                    .transfer_syntax("1.2.840.10008.1.2.1")
                    .media_storage_sop_class_uid("1.2.840.10008.5.1.4.1.1.7")
                    .media_storage_sop_instance_uid("1.2.3.4"),
            )
            .expect("file meta group")
            .write_all(&mut no_uids)
            .expect("encode the Part 10 file");

        let rejected = [
            (serde_json::json!(7), "not a base64 string"),
            (serde_json::json!("not base64!"), "base64 decode failed"),
            (
                serde_json::json!(b64(b"bare bytes")),
                "not a DICOM Part 10 file",
            ),
            (serde_json::json!(b64(&no_uids)), "no SOP Class UID"),
        ];
        for (part, expected) in rejected {
            match decode_stow_part(&part) {
                Ok(_) => panic!("{} should be rejected", part),
                Err(reason) => {
                    assert!(
                        reason.contains(expected),
                        "expected {:?} in: {}",
                        expected,
                        reason
                    )
                }
            }
        }
    }

    #[test]
    fn test_decode_stow_part_reads_both_uids() {
        let instance = decode_stow_part(&serde_json::json!(b64(&sample_part10())))
            .expect("sample instance is accepted");

        assert_eq!(instance.sop_instance_uid, sample_sop_instance_uid());
        assert!(!instance.sop_class_uid.is_empty());
    }

    #[test]
    fn test_store_dataset_sends_other_bytes_as_a_bare_data_set() {
        // (0008,0016) SOP Class UID, Implicit VR Little Endian. Not Part 10
        // and not JSON.
        let bare = [0x08, 0x00, 0x16, 0x00, 0x02, 0x00, 0x00, 0x00, b'1', 0x00];

        let dataset = DicomScuBackend::store_dataset_from_body(&bare).expect("data set");

        match &dataset {
            DatasetStream::Memory { data, .. } => assert_eq!(data.as_ref(), &bare[..]),
            other => panic!(
                "expected the body as bare bytes, got {:?}",
                other.metadata()
            ),
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
            (
                json_body(serde_json::json!({"dataset": {}})),
                "no 'part10_b64' string",
            ),
            (
                json_body(serde_json::json!({"part10_b64": 7})),
                "no 'part10_b64' string",
            ),
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
            assert!(
                reason.contains(expected),
                "expected {:?} in: {}",
                expected,
                reason
            );
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

    fn study_identifier(study_uid: &str) -> DatasetStream {
        use dicom_core::{DataElement, PrimitiveValue, VR};
        let mut object = dicom_object::InMemDicomObject::new_empty();
        object.put(DataElement::new(
            dicom_dictionary_std::tags::STUDY_INSTANCE_UID,
            VR::UI,
            PrimitiveValue::from(study_uid),
        ));
        DatasetStream::from_object(object)
    }

    #[test]
    fn test_dimse_failure_response_hides_the_raw_error() {
        let raw = "association to PACS_AET at 10.1.2.3:104 refused";
        for (op, message) in [
            ("find", "C-FIND failed"),
            ("get", "C-GET failed"),
            ("move", "C-MOVE failed"),
        ] {
            let response = dimse_failure_response(op, &raw);
            assert_eq!(response["operation"], op);
            assert_eq!(response["success"], false);
            assert_eq!(response["error"], message);
            assert!(!response.to_string().contains("10.1.2.3"));
        }
    }

    #[tokio::test]
    async fn test_collect_find_matches_returns_all_matches() {
        let stream = futures_util::stream::iter(vec![
            Ok(study_identifier("1.2.3")),
            Ok(study_identifier("1.2.4")),
        ]);

        let matches = collect_find_matches(stream).await.unwrap();

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0]["0020000D"]["Value"][0], "1.2.3");
    }

    #[tokio::test]
    async fn test_collect_find_matches_fails_on_an_error_after_some_matches() {
        let stream = futures_util::stream::iter(vec![
            Ok(study_identifier("1.2.3")),
            Err(dimse::DimseError::network("connection reset")),
        ]);

        assert!(collect_find_matches(stream).await.is_err());
    }

    #[test]
    fn test_move_preflight_proceeds_when_the_study_is_found() {
        assert!(matches!(
            move_preflight_outcome(Some(Ok(study_identifier("1.2.3")))),
            MovePreflight::Proceed
        ));
    }

    #[test]
    fn test_move_preflight_reports_a_missing_study() {
        assert!(matches!(
            move_preflight_outcome(None),
            MovePreflight::StudyNotFound
        ));
    }

    #[test]
    fn test_move_preflight_stops_on_a_connection_failure() {
        let outcome = move_preflight_outcome(Some(Err(dimse::DimseError::association(
            "Failed to establish association: could not connect to server",
        ))));
        assert!(matches!(outcome, MovePreflight::Unreachable(_)));
    }

    #[test]
    fn test_move_preflight_proceeds_when_c_find_is_refused_but_the_node_answered() {
        for error in [
            dimse::DimseError::NotSupported("no presentation context for C-FIND".to_string()),
            dimse::DimseError::operation_failed("C-FIND failed with status: 0xC000"),
        ] {
            assert!(matches!(
                move_preflight_outcome(Some(Err(error))),
                MovePreflight::Proceed
            ));
        }
    }
}
