use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
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
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct DicomEndpoint {
    pub local_aet: Option<String>,
    pub aet: Option<String>, // For backward compatibility (remote AET)
    pub host: Option<String>,
    pub port: Option<u16>,
    pub use_tls: Option<bool>,
}

impl DicomEndpoint {
    /// Check if this is being used as a backend (SCU) vs endpoint (SCP)
    fn is_backend_usage(&self, options: &HashMap<String, Value>) -> bool {
        // If host/aet are provided, it's for backend usage (connecting to remote)
        // Note: 'port' alone can be used for SCP listener and should NOT imply backend usage
        options.contains_key("host") || options.contains_key("aet")
    }

    /// Get the local AET from options or struct
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
        let aet = options
            .get("aet")
            .and_then(|v| v.as_str())
            .or(self.aet.as_deref())
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Missing remote 'aet' (Application Entity Title)".to_string(),
            })?
            .to_string();

        let host = options
            .get("host")
            .and_then(|v| v.as_str())
            .or(self.host.as_deref())
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Missing 'host' (DICOM server address)".to_string(),
            })?
            .to_string();

        let port = options
            .get("port")
            .and_then(|v| v.as_u64())
            .or(self.port.map(|p| p as u64))
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Missing 'port'".to_string(),
            })?;

        // DICOM servers commonly use privileged ports like 104, so allow 1-65535 for remote nodes
        if !(1..=65535).contains(&port) {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
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
impl ServiceType for DicomEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        if self.is_backend_usage(options) {
            // Backend usage - validate remote connection parameters
            self.create_remote_node(options)?;
        } else {
            // Endpoint usage - validate local AET only for SCP listener
            let local_aet =
                self.get_local_aet(options)
                    .ok_or_else(|| ConfigError::InvalidEndpoint {
                        name: "dicom".to_string(),
                        reason: "Missing 'local_aet' for DICOM endpoint (SCP)".to_string(),
                    })?;

            if local_aet.trim().is_empty() || local_aet.len() > 16 {
                return Err(ConfigError::InvalidEndpoint {
                    name: "dicom".to_string(),
                    reason: "Local AE title must be 1-16 characters".to_string(),
                });
            }

            // Optional: validate port if provided
            if let Some(port_val) = options.get("port").and_then(|v| v.as_u64()) {
                if port_val == 0 || port_val > 65535 {
                    return Err(ConfigError::InvalidEndpoint {
                        name: "dicom".to_string(),
                        reason: "Invalid 'port' (Allowed range: 1-65535)".to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        if self.is_backend_usage(options) {
            // Backend usage - no HTTP routes needed (DIMSE protocol only)
            vec![]
        } else {
            // Endpoint usage - no HTTP routes; SCP listener is started by the router/dispatcher with pipeline context
            vec![]
        }
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        use crate::models::envelope::envelope::{RequestDetails, RequestEnvelope};
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
        let uri = format!("dicom://scp/{}", op.to_lowercase());
        let details = RequestDetails {
            method: op,
            uri,
            headers: Map::new(),
            cookies: Map::new(),
            query_params: Map::new(),
            cache_status: None,
            metadata,
        };

        // Prefer normalized_data as the JSON body if payload is JSON
        let normalized: Option<serde_json::Value> = serde_json::from_slice(&ctx.payload).ok();

        Ok(RequestEnvelope {
            request_details: details,
            original_data: ctx.payload,
            normalized_data: normalized,
            normalized_snapshot: None,
        })
    }
}

#[async_trait]
impl ServiceHandler<Value> for DicomEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        if self.is_backend_usage(options) {
            // Backend usage - prepare for DIMSE SCU operations
            self.handle_backend_request(&mut envelope, options).await
        } else {
            // Misconfiguration: DICOM cannot act as HTTP endpoint
            Err(Error::from("DICOM service cannot be used as an endpoint; configure an HTTP endpoint and a DICOM backend instead"))
        }
    }

    async fn transform_response(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        let nd = envelope.normalized_data.unwrap_or(serde_json::Value::Null);
        let response_meta = nd.get("response");

        let status = response_meta
            .and_then(|m| m.get("status"))
            .and_then(|s| s.as_u64())
            .and_then(|code| http::StatusCode::from_u16(code as u16).ok())
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);
        let mut has_content_type = false;
        if let Some(hdrs) = response_meta
            .and_then(|m| m.get("headers"))
            .and_then(|h| h.as_object())
        {
            for (k, v) in hdrs.iter() {
                if let Some(val_str) = v.as_str() {
                    if k.eq_ignore_ascii_case("content-type") {
                        has_content_type = true;
                    }
                    builder = builder.header(k.as_str(), val_str);
                }
            }
        }

        if let Some(body_str) = response_meta
            .and_then(|m| m.get("body"))
            .and_then(|b| b.as_str())
        {
            return builder
                .body(Body::from(body_str.to_string()))
                .map_err(|_| Error::from("Failed to construct DICOM HTTP response"));
        }

        let body_str = serde_json::to_string(&nd)
            .map_err(|_| Error::from("Failed to serialize DICOM response payload into JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct DICOM HTTP response"))
    }
}

impl DicomEndpoint {
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

        // Extract path for context and resolve operation (prefer dimse_op set by middleware)
        let path = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();
        let op = envelope
            .request_details
            .metadata
            .get("dimse_op")
            .cloned()
            .unwrap_or_else(|| path.clone());

        let result = match op.as_str() {
            "echo" | "/echo" => {
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
            "find" | "/find" => {
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
                        // Expect { vr: ..., Value: [...] }
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
                        }
                    }
                }

                // Choose query level (default: Patient)
                let mut query = FindQuery::patient(params.get("00100020").cloned()); // PatientID if present
                query.query_level = QueryLevel::Patient;
                for (k, v) in params.into_iter() {
                    query = query.with_parameter(k, v);
                }

                // Perform C-FIND and collect results
                match scu.find(&remote_node, query).await {
                    Ok(mut stream) => {
                        use futures_util::StreamExt;
                        let mut matches: Vec<serde_json::Value> = Vec::new();
                        while let Some(item) = stream.next().await {
                            if let Ok(dimse::types::DatasetStream::File { ref path, .. }) = item {
                                if let Ok(obj) = dicom_object::open_file(path) {
                                    if let Ok(json) =
                                        dicom_json_tool::identifier_to_json_value(&obj)
                                    {
                                        matches.push(json);
                                    }
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
            "move" | "/move" => {
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
                                // Return 404 early
                                let mut hdrs = HashMap::new();
                                hdrs.insert(
                                    "content-type".to_string(),
                                    "application/json".to_string(),
                                );
                                let body =
                                    serde_json::json!({"error":"Study not found"}).to_string();
                                let mut resp = serde_json::Map::new();
                                resp.insert("status".into(), serde_json::json!(404u16));
                                resp.insert("headers".into(), serde_json::json!(hdrs));
                                resp.insert("body".into(), serde_json::json!(body));
                                envelope.normalized_data = Some(
                                    serde_json::json!({"response": serde_json::Value::Object(resp)}),
                                );
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

                // In persistent SCP mode, create a per-move subdirectory and direct the SCP to use it
                let mut per_move_dir_opt: Option<std::path::PathBuf> = None;
                if persistent_scp {
                    let scp_root = options
                        .get("storage_dir")
                        .and_then(|v| v.as_str())
                        .unwrap_or("./tmp/dimse");
                    let per_move_dir = std::path::Path::new(scp_root).join(&folder_id);
                    let _ = std::fs::create_dir_all(&per_move_dir);
                    per_move_dir_opt = Some(per_move_dir.clone());
                    // Set the current store dir for the persistent SCP QueryProvider (internal SCP)
                    crate::integrations::dimse::pipeline_query_provider::set_current_store_dir(
                        per_move_dir.clone(),
                    );
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
                                        let bytes = tokio::fs::read(path).await.unwrap_or_else(|_| Vec::new());
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
                                                let _ =
                                                    std::fs::rename(p, &target).or_else(|_| {
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
            "get" | "/get" => {
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
                                        let bytes = tokio::fs::read(path).await.unwrap_or_else(|_| Vec::new());
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
            _ => serde_json::json!({
                "operation": "unknown",
                "success": false,
                "error": format!("Unknown DIMSE operation: {}", path)
            }),
        };

        envelope.normalized_data = Some(result);
        Ok(envelope.clone())
    }
}
