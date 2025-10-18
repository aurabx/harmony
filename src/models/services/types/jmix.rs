use crate::config::config::ConfigError;
use crate::file;
use crate::globals::get_storage;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
pub struct JmixEndpoint {}

#[async_trait]
impl ServiceType for JmixEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is non-empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(ConfigError::InvalidEndpoint {
                name: "jmix".to_string(),
                reason: "Jmix endpoint requires a non-empty 'path_prefix'".to_string(),
            });
        }

        // Validate skip_hashing option if provided
        if let Some(skip_hashing) = options.get("skip_hashing") {
            if !skip_hashing.is_boolean() {
                return Err(ConfigError::InvalidEndpoint {
                    name: "jmix".to_string(),
                    reason: "skip_hashing must be a boolean value".to_string(),
                });
            }
        }

        // Validate skip_listing option if provided
        if let Some(skip_listing) = options.get("skip_listing") {
            if !skip_listing.is_boolean() {
                return Err(ConfigError::InvalidEndpoint {
                    name: "jmix".to_string(),
                    reason: "skip_listing must be a boolean value".to_string(),
                });
            }
        }

        // Validate dimse_operation option if provided
        if let Some(dimse_op) = options.get("dimse_operation") {
            if let Some(op_str) = dimse_op.as_str() {
                let op_lower = op_str.to_lowercase();
                if op_lower != "get" && op_lower != "move" {
                    return Err(ConfigError::InvalidEndpoint {
                        name: "jmix".to_string(),
                        reason: "dimse_operation must be either 'get' or 'move'".to_string(),
                    });
                }
            } else {
                return Err(ConfigError::InvalidEndpoint {
                    name: "jmix".to_string(),
                    reason: "dimse_operation must be a string value".to_string(),
                });
            }
        }

        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // JMIX exposes a fixed set of API routes. We register only those here and do NOT add any catch-all.
        // Unknown paths under the prefix will 404 automatically.
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("/jmix");
        let base = path_prefix.trim_end_matches('/');

        let mut routes: Vec<RouteConfig> = vec![
            // JMIX Endpoint API (strict route set)
            // 1a. GET Envelope by ID
            RouteConfig {
                path: format!("{}/api/jmix/{{id}}", base),
                methods: vec![Method::GET],
                description: Some("JMIX get envelope by id".to_string()),
            },
            // 1b. GET manifest by ID
            RouteConfig {
                path: format!("{}/api/jmix/{{id}}/manifest", base),
                methods: vec![Method::GET],
                description: Some("JMIX get manifest by id".to_string()),
            },
            // 1c. GET by query (e.g., studyInstanceUid)
            RouteConfig {
                path: format!("{}/api/jmix", base),
                methods: vec![Method::GET],
                description: Some("JMIX query envelope".to_string()),
            },
            // 2. POST Envelope upload
            RouteConfig {
                path: format!("{}/api/jmix", base),
                methods: vec![Method::POST],
                description: Some("JMIX upload envelope".to_string()),
            },
        ];

        // Optionally allow OPTIONS preflight and HEAD method automatically on GET routes
        // to avoid duplicate handlers when multiple methods share the same path.
        routes = routes
            .into_iter()
            .map(|mut rc| {
                if rc.methods.contains(&http::Method::GET) {
                    if !rc.methods.contains(&http::Method::OPTIONS) {
                        rc.methods.push(http::Method::OPTIONS);
                    }
                    if !rc.methods.contains(&http::Method::HEAD) {
                        rc.methods.push(http::Method::HEAD);
                    }
                }
                rc
            })
            .collect();

        routes
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        // For HTTP protocol, delegate to HttpEndpoint for consistent HTTP parsing
        if ctx.protocol == crate::models::protocol::Protocol::Http {
            let http = crate::models::services::types::http::HttpEndpoint {};
            return http.build_protocol_envelope(ctx, options).await;
        }
        Err(crate::utils::Error::from(
            "JmixEndpoint only supports Protocol::Http envelope building",
        ))
    }
}

fn find_package_root_and_manifest(
    extracted_root: &Path,
) -> Result<(PathBuf, serde_json::Value), Error> {
    // Look for manifest.json; choose dir containing it as package root
    for entry in WalkDir::new(extracted_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() && entry.file_name() == "manifest.json" {
            let pkg_dir = entry.path().parent().unwrap().to_path_buf();
            let s = fs::read_to_string(entry.path())
                .map_err(|e| Error::from(format!("manifest read error: {}", e)))?;
            let json: serde_json::Value = serde_json::from_str(&s)
                .map_err(|e| Error::from(format!("manifest parse error: {}", e)))?;
            return Ok((pkg_dir, json));
        }
    }
    Err(Error::from("manifest.json not found in uploaded archive"))
}

#[async_trait]
impl ServiceHandler<Value> for JmixEndpoint {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        let method = envelope.request_details.method.to_uppercase();
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();

        tracing::debug!("JMIX SERVICE: method='{}', subpath='{}'", method, subpath);

        // Pass endpoint options to middleware via metadata
        envelope
            .request_details
            .metadata
            .insert("jmix_method".to_string(), method.clone());
        if let Some(store_dir) = options.get("store_dir").and_then(|v| v.as_str()) {
            envelope
                .request_details
                .metadata
                .insert("endpoint_store_dir".to_string(), store_dir.to_string());
        }

        // Route matching
        // GET/HEAD /api/jmix/{id}
        if (method == "GET" || method == "HEAD")
            && subpath.starts_with("api/jmix/")
            && !subpath.ends_with("/manifest")
        {
            let rest = &subpath["api/jmix/".len()..];
            if !rest.is_empty() && !rest.contains('/') {
                let id = rest;
                // Set metadata for middleware to handle
                envelope
                    .request_details
                    .metadata
                    .insert("jmix_id".to_string(), id.to_string());
                // Let middleware check if package exists and serve it
                return Ok(envelope);
            }
        }

        // GET/HEAD /api/jmix/{id}/manifest
        if (method == "GET" || method == "HEAD")
            && subpath.starts_with("api/jmix/")
            && subpath.ends_with("/manifest")
        {
            let rest = &subpath["api/jmix/".len()..];
            if let Some(id_part) = rest.strip_suffix("/manifest") {
                let id = id_part.trim_end_matches('/');
                if !id.is_empty() && !id.contains('/') {
                    // Set metadata for middleware to handle
                    envelope
                        .request_details
                        .metadata
                        .insert("jmix_id".to_string(), id.to_string());
                    envelope
                        .request_details
                        .metadata
                        .insert("jmix_wants_manifest".to_string(), "true".to_string());
                    return Ok(envelope);
                }
            }
        }

        // GET/HEAD /api/jmix?studyInstanceUid=...
        if (method == "GET" || method == "HEAD") && subpath == "api/jmix" {
            // Extract studyInstanceUid
            let study_uid = envelope
                .request_details
                .query_params
                .get("studyInstanceUid")
                .and_then(|v| v.first())
                .map(|s| s.to_string());

            if let Some(uid) = study_uid {
                // Set metadata for middleware to check
                envelope
                    .request_details
                    .metadata
                    .insert("jmix_study_uid".to_string(), uid.clone());

                // Prepare DICOM identifier for backend in case middleware doesn't find local match
                let identifier = serde_json::json!({
                    "0020000D": { "vr": "UI", "Value": [ uid ] }
                });
                envelope.original_data = serde_json::to_vec(&identifier)
                    .map_err(|e| Error::from(format!("identifier encode error: {}", e)))?;

                // Extract skip flags from config (defaults) and query parameters (overrides)
                let config_skip_hashing = options
                    .get("skip_hashing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let config_skip_listing = options
                    .get("skip_listing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let skip_hashing = envelope
                    .request_details
                    .query_params
                    .get("skip_hashing")
                    .and_then(|v| v.first())
                    .and_then(|s| s.parse::<bool>().ok())
                    .unwrap_or(config_skip_hashing);
                let skip_listing = envelope
                    .request_details
                    .query_params
                    .get("skip_listing")
                    .and_then(|v| v.first())
                    .and_then(|s| s.parse::<bool>().ok())
                    .unwrap_or(config_skip_listing);

                // Pass skip flags to middleware via metadata
                if skip_hashing {
                    envelope
                        .request_details
                        .metadata
                        .insert("skip_hashing".to_string(), "true".to_string());
                }
                if skip_listing {
                    envelope
                        .request_details
                        .metadata
                        .insert("skip_listing".to_string(), "true".to_string());
                }

                // Determine which DICOM operation to use (C-GET or C-MOVE)
                // Default to C-GET if not specified, as it works without PACS-side AE configuration
                // C-MOVE requires the PACS to be configured with the SCU's AE title and network address
                // C-GET receives images directly without requiring bidirectional network setup
                let dimse_operation = options
                    .get("dimse_operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("get")
                    .to_lowercase();
                
                envelope
                    .request_details
                    .metadata
                    .insert("dimse_op".to_string(), dimse_operation.clone());
                envelope
                    .request_details
                    .metadata
                    .insert("path".to_string(), dimse_operation);

                // Let middleware check if local JMIX exists; if not, backends will handle it
                return Ok(envelope);
            } else {
                let mut resp = serde_json::Map::new();
                resp.insert("status".to_string(), serde_json::json!(400));
                resp.insert(
                    "body".to_string(),
                    serde_json::json!("missing studyInstanceUid"),
                );
                envelope.normalized_data =
                    Some(serde_json::json!({"response": serde_json::Value::Object(resp)}));
                envelope
                    .request_details
                    .metadata
                    .insert("skip_backends".to_string(), "true".to_string());
                return Ok(envelope);
            }
        }

        // POST /api/jmix (upload envelope)
        if method == "POST" && subpath == "api/jmix" {
            let headers = &envelope.request_details.headers;
            let content_type = headers
                .get("content-type")
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            let is_zip = content_type.contains("zip");

            if !is_zip {
                let mut resp = serde_json::Map::new();
                resp.insert("status".to_string(), serde_json::json!(415));
                resp.insert(
                    "body".to_string(),
                    serde_json::json!(
                        "Only application/zip content type is supported for JMIX upload"
                    ),
                );
                envelope.normalized_data =
                    Some(serde_json::json!({"response": serde_json::Value::Object(resp)}));
                envelope
                    .request_details
                    .metadata
                    .insert("skip_backends".to_string(), "true".to_string());
                return Ok(envelope);
            }

            // Create temp dir for upload extraction using storage backend
            let temp_extract_dir = if let Some(storage) = get_storage() {
                storage
                    .tempdir_in_str("jmix-upload", "jmix_upload_")
                    .map_err(|e| Error::from(format!("storage tempdir error: {}", e)))?
            } else {
                // Fallback to manual creation if storage not available
                let tmp_root = Path::new("./tmp").join("jmix-upload");
                let _ = fs::create_dir_all(&tmp_root);
                tempfile::Builder::new()
                    .prefix("jmix_upload_")
                    .tempdir_in(&tmp_root)
                    .map_err(|e| Error::from(format!("tempdir error: {}", e)))?
            };

            // Extract zip archive
            let extracted_root = temp_extract_dir.path().to_path_buf();
            let body = &envelope.original_data;
            let extract_res = file::extract_zip(body, &extracted_root);

            if let Err(e) = extract_res {
                let mut resp = serde_json::Map::new();
                resp.insert("status".to_string(), serde_json::json!(415));
                resp.insert(
                    "body".to_string(),
                    serde_json::json!(format!("extract error: {}", e)),
                );
                envelope.normalized_data =
                    Some(serde_json::json!({"response": serde_json::Value::Object(resp)}));
                envelope
                    .request_details
                    .metadata
                    .insert("skip_backends".to_string(), "true".to_string());
                return Ok(envelope);
            }

            // Resolve store root
            let store_root = if let Some(p) = options.get("store_dir").and_then(|v| v.as_str()) {
                PathBuf::from(p)
            } else if let Some(storage) = get_storage() {
                storage.subpath_str("jmix-store")
            } else {
                PathBuf::from("./tmp/jmix-store")
            };

            // Find manifest.json in extracted content
            let (pkg_dir, manifest_json) = find_package_root_and_manifest(&extracted_root)?;
            let id = manifest_json
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if id.is_empty() {
                let mut resp = serde_json::Map::new();
                resp.insert("status".to_string(), serde_json::json!(400));
                resp.insert("body".to_string(), serde_json::json!("manifest.id missing"));
                envelope.normalized_data =
                    Some(serde_json::json!({"response": serde_json::Value::Object(resp)}));
                envelope
                    .request_details
                    .metadata
                    .insert("skip_backends".to_string(), "true".to_string());
                return Ok(envelope);
            }

            // Move into store_root/{id}
            let dest_dir = store_root.join(id);
            if dest_dir.exists() {
                let mut resp = serde_json::Map::new();
                resp.insert("status".to_string(), serde_json::json!(409));
                resp.insert(
                    "body".to_string(),
                    serde_json::json!("envelope id already exists"),
                );
                envelope.normalized_data =
                    Some(serde_json::json!({"response": serde_json::Value::Object(resp)}));
                envelope
                    .request_details
                    .metadata
                    .insert("skip_backends".to_string(), "true".to_string());
                return Ok(envelope);
            }
            fs::create_dir_all(&store_root)
                .map_err(|e| Error::from(format!("store dir error: {}", e)))?;
            fs_extra::dir::copy(
                &pkg_dir,
                &store_root,
                &fs_extra::dir::CopyOptions {
                    copy_inside: true,
                    content_only: false,
                    overwrite: false,
                    ..Default::default()
                },
            )
            .map_err(|e| Error::from(format!("store copy error: {}", e)))?;
            // If copied as <tmp>/<id>.jmix into store_root/<id>.jmix? Our copy may result in store_root/<id>.jmix. Ensure correct location.
            // Validate with jmix-rs
            let opts = jmix_rs::ValidationOptions {
                schema_dir: None,
                validate_schema: true,
                verify_assertions: false,
                recipient_secret_key_path: None,
            };
            match jmix_rs::validate_package(&dest_dir, &opts) {
                Ok(_report) => {
                    let mut resp = serde_json::Map::new();
                    resp.insert("status".to_string(), serde_json::json!(201));
                    let mut hdrs = HashMap::new();
                    hdrs.insert("content-type".to_string(), "application/json".to_string());
                    resp.insert("headers".to_string(), serde_json::json!(hdrs));
                    let json = serde_json::json!({ "id": id, "status": "stored" });
                    resp.insert("json".to_string(), json);
                    envelope.normalized_data = Some(serde_json::json!({
                        "response": serde_json::Value::Object(resp)
                    }));
                }
                Err(e) => {
                    // Cleanup on failure
                    let _ = fs::remove_dir_all(&dest_dir);
                    let mut resp = serde_json::Map::new();
                    resp.insert("status".to_string(), serde_json::json!(400));
                    resp.insert(
                        "body".to_string(),
                        serde_json::json!(format!("validation failed: {}", e)),
                    );
                    envelope.normalized_data =
                        Some(serde_json::json!({"response": serde_json::Value::Object(resp)}));
                }
            }
            // Prevent forwarding to backends
            envelope
                .request_details
                .metadata
                .insert("skip_backends".to_string(), "true".to_string());
            return Ok(envelope);
        }

        // Fallback: 404 for any other JMIX path not handled above
        let mut resp = serde_json::Map::new();
        resp.insert("status".to_string(), serde_json::json!(404));
        resp.insert("body".to_string(), serde_json::json!("Not Found"));
        envelope.normalized_data =
            Some(serde_json::json!({"response": serde_json::Value::Object(resp)}));
        envelope
            .request_details
            .metadata
            .insert("skip_backends".to_string(), "true".to_string());
        Ok(envelope)
    }

    /// @todo This could be incorrect. Check.
    async fn backend_outgoing_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // JMIX service prepares response in endpoint_incoming_request
        // Extract response metadata from request_details.metadata (set by middleware)
        let status = envelope
            .request_details
            .metadata
            .get("jmix_response_status")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(200);

        // Extract headers from metadata
        let mut headers = HashMap::new();
        if let Some(headers_json) = envelope
            .request_details
            .metadata
            .get("jmix_response_headers")
        {
            if let Ok(hdrs) = serde_json::from_str::<HashMap<String, String>>(headers_json) {
                headers = hdrs;
            }
        }

        // Build body from normalized_data (set by middleware)
        let body = if let Some(ref normalized) = envelope.normalized_data {
            serde_json::to_vec(normalized).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Ensure content-type is set if not present
        if !headers.contains_key("content-type") && !headers.contains_key("Content-Type") {
            headers.insert("content-type".to_string(), "application/json".to_string());
        }

        // Copy JMIX metadata from request to response
        let mut response_metadata = HashMap::new();
        if let Some(jmix_id) = envelope.request_details.metadata.get("jmix_id") {
            response_metadata.insert("jmix_id".to_string(), jmix_id.clone());
        }
        if let Some(zip_ready) = envelope.request_details.metadata.get("jmix_zip_ready") {
            response_metadata.insert("jmix_zip_ready".to_string(), zip_ready.clone());
        }

        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            headers,
            body,
            Some(response_metadata),
        );

        // Preserve normalized_data
        response_envelope.normalized_data = envelope.normalized_data;

        Ok(response_envelope)
    }

    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Add protocol metadata for JMIX service
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));
        envelope
            .response_details
            .metadata
            .insert("service".to_string(), "jmix".to_string());
        
        // For HTTP protocol, ensure JMIX content-type if applicable
        if ctx.protocol == crate::models::protocol::Protocol::Http {
            // Only set if not already a zip file
            if !envelope.response_details.headers.contains_key("content-type") {
                envelope
                    .response_details
                    .headers
                    .insert("content-type".to_string(), "application/json".to_string());
            }
        }
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Check if response has jmix metadata for special zip file handling
        // Only serve zip if BOTH jmix_id and jmix_zip_ready are set
        // Check both response and request metadata (fallback for when backends are skipped)
        let zip_ready = envelope
            .response_details
            .metadata
            .get("jmix_zip_ready")
            .or_else(|| envelope.request_details.metadata.get("jmix_zip_ready"))
            .map(|s| s == "true")
            .unwrap_or(false);

        if zip_ready {
            let jmix_id = envelope
                .response_details
                .metadata
                .get("jmix_id")
                .or_else(|| envelope.request_details.metadata.get("jmix_id"));
            if let Some(jmix_id) = jmix_id {
                // Load the zip file and return it (special case for JMIX)
                tracing::info!("📦 Serving zip file for JMIX package: {}", jmix_id);

                let store_root = if let Some(p) = options.get("store_dir").and_then(|v| v.as_str())
                {
                    PathBuf::from(p)
                } else if let Some(storage) = get_storage() {
                    storage.subpath_str("jmix-store")
                } else {
                    PathBuf::from("./tmp/jmix-store")
                };

                let package_dir = store_root.join(jmix_id);
                let zip_file = package_dir.join(format!("{}.zip", jmix_id));

                if zip_file.exists() {
                    match fs::read(&zip_file) {
                        Ok(zip_bytes) => {
                            let filename = format!("{}.zip", jmix_id);
                            return Response::builder()
                                .status(http::StatusCode::OK)
                                .header("content-type", "application/zip")
                                .header(
                                    "content-disposition",
                                    format!("attachment; filename=\"{}\"", filename),
                                )
                                .body(Body::from(zip_bytes))
                                .map_err(|_| Error::from("Failed to construct zip response"));
                        }
                        Err(e) => {
                            tracing::error!(
                                "❌ Failed to read zip file {}: {}",
                                zip_file.display(),
                                e
                            );
                            return Response::builder()
                                .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                                .body(Body::from(format!("Failed to read zip file: {}", e)))
                                .map_err(|_| Error::from("Failed to construct error response"));
                        }
                    }
                } else {
                    tracing::error!("⚠️ Zip file not found: {}", zip_file.display());
                    return Response::builder()
                        .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("JMIX zip file not found"))
                        .map_err(|_| Error::from("Failed to construct error response"));
                }
            }
        }

        // Standard ResponseEnvelope handling
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
                .map_err(|_| Error::from("Failed to serialize JMIX response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct JMIX HTTP response"))
    }
}
