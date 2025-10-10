use crate::config::config::ConfigError;
use crate::globals::get_storage;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use base64::Engine;
use flate2::write::GzEncoder;
use flate2::Compression;
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use tar::Builder as TarBuilder;
use walkdir::WalkDir;
use zip::write::FileOptions as ZipFileOptions;
use zip::ZipWriter;

// jmix-rs types for optional parsing
use jmix_rs::types::Metadata as JmixMetadata;

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

fn resolve_store_root(options: &HashMap<String, Value>) -> PathBuf {
    if let Some(p) = options.get("store_dir").and_then(|v| v.as_str()) {
        return PathBuf::from(p);
    }

    // Use global storage backend with jmix-store subdirectory
    if let Some(storage) = get_storage() {
        return storage.subpath_str("jmix-store");
    }

    // Fallback to ./tmp/jmix-store if storage not available
    PathBuf::from("./tmp/jmix-store")
}

fn package_dir_for(store_root: &Path, id: &str) -> PathBuf {
    store_root.join(format!("{}.jmix", id))
}

fn make_targz(dir: &Path) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    {
        let gz = GzEncoder::new(&mut buf, Compression::default());
        let mut tar = TarBuilder::new(gz);
        tar.append_dir_all(".", dir)
            .map_err(|e| Error::from(format!("tar error: {}", e)))?;
        tar.finish()
            .map_err(|e| Error::from(format!("tar finish error: {}", e)))?;
    } // drop tar and gz encoder to flush into buf
    Ok(buf)
}

fn make_zip(dir: &Path) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    {
        let cursor = Cursor::new(&mut buf);
        let mut zip = ZipWriter::new(cursor);
        let options =
            ZipFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let base = dir;
        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap_or(path);
            let name = rel.to_string_lossy();
            if entry.file_type().is_dir() {
                if !name.is_empty() {
                    zip.add_directory(name.to_string(), options)
                        .map_err(|e| Error::from(format!("zip dir error: {}", e)))?;
                }
            } else if entry.file_type().is_file() {
                zip.start_file(name.to_string(), options)
                    .map_err(|e| Error::from(format!("zip file error: {}", e)))?;
                let data =
                    fs::read(path).map_err(|e| Error::from(format!("zip read error: {}", e)))?;
                zip.write_all(&data)
                    .map_err(|e| Error::from(format!("zip write error: {}", e)))?;
            }
        }
        zip.finish()
            .map_err(|e| Error::from(format!("zip finish error: {}", e)))?;
    }
    Ok(buf)
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), Error> {
    let reader = Cursor::new(bytes);
    let mut zip =
        zip::ZipArchive::new(reader).map_err(|e| Error::from(format!("zip open error: {}", e)))?;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| Error::from(format!("zip idx error: {}", e)))?;
        let outpath = dest.join(file.mangled_name());
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| Error::from(format!("mkdir error: {}", e)))?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| Error::from(format!("mkparent error: {}", e)))?;
            }
            let mut outfile = fs::File::create(&outpath)
                .map_err(|e| Error::from(format!("create error: {}", e)))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| Error::from(format!("write error: {}", e)))?;
        }
    }
    Ok(())
}

fn extract_targz(bytes: &[u8], dest: &Path) -> Result<(), Error> {
    let gz = flate2::read::GzDecoder::new(Cursor::new(bytes));
    let mut ar = tar::Archive::new(gz);
    ar.unpack(dest)
        .map_err(|e| Error::from(format!("targz unpack error: {}", e)))
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

fn query_by_study_uid(store_root: &Path, study_uid: &str) -> Result<Vec<serde_json::Value>, Error> {
    let mut results = Vec::new();
    if !store_root.exists() {
        return Ok(results);
    }
    for entry in
        fs::read_dir(store_root).map_err(|e| Error::from(format!("readdir error: {}", e)))?
    {
        let entry = entry.map_err(|e| Error::from(format!("direntry error: {}", e)))?;
        let path = entry.path();
        if path.is_dir() && path.extension().and_then(|e| e.to_str()) == Some("jmix") {
            let metadata_path = path.join("payload").join("metadata.json");
            if metadata_path.exists() {
                if let Ok(s) = fs::read_to_string(&metadata_path) {
                    if let Ok(md) = serde_json::from_str::<JmixMetadata>(&s) {
                        if let Some(studies) = md.studies {
                            if let Some(uid) = studies.study_uid {
                                if uid == study_uid {
                                    results.push(serde_json::json!({
                                        "id": md.id,
                                        "path": path.to_string_lossy().to_string(),
                                        "studyInstanceUid": uid
                                    }));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(results)
}

#[async_trait]
impl ServiceHandler<Value> for JmixEndpoint {
    type ReqBody = Value;

    async fn transform_request(
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
        let headers = &envelope.request_details.headers;
        let accept = headers.get("accept").map(|s| s.as_str());
        
        tracing::debug!("JMIX SERVICE: method='{}', subpath='{}', accept='{:?}'", method, subpath, accept);

        // Helper: set response meta into normalized_data
        let mut set_response = |status: http::StatusCode,
                                hdrs: HashMap<String, String>,
                                body_str: Option<String>,
                                json_obj: Option<serde_json::Value>,
                                body_b64: Option<String>| {
            let mut resp = serde_json::Map::new();
            resp.insert("status".to_string(), serde_json::json!(status.as_u16()));
            if !hdrs.is_empty() {
                resp.insert("headers".to_string(), serde_json::json!(hdrs));
            }
            if let Some(s) = body_str {
                resp.insert("body".to_string(), serde_json::json!(s));
            }
            if let Some(j) = json_obj {
                resp.insert("json".to_string(), j);
            }
            if let Some(b) = body_b64 {
                resp.insert("body_b64".to_string(), serde_json::json!(b));
            }
            envelope.normalized_data = Some(serde_json::json!({
                "response": serde_json::Value::Object(resp)
            }));
        };

        // Resolve storage root (default to ./tmp/jmix-store per project rule)
        let store_root = resolve_store_root(options);

        // Route matching
        // GET/HEAD /api/jmix/{id}
        if (method == "GET" || method == "HEAD") && subpath.starts_with("api/jmix/") && !subpath.ends_with("/manifest") {
            let rest = &subpath["api/jmix/".len()..];
            if !rest.is_empty() && !rest.contains('/') {
                let id = rest;
                // Negotiate Accept
                let wants_gzip = accept
                    .map(|a| a.contains("application/gzip") || a.contains("application/x-gtar"))
                    .unwrap_or(false);
                let wants_zip = accept
                    .map(|a| a.contains("application/zip"))
                    .unwrap_or(false);
                let negotiated =
                    if accept.is_none() || wants_zip || (!wants_gzip && accept == Some("*/*")) {
                        Some("application/zip")
                    } else if wants_gzip {
                        Some("application/gzip")
                    } else {
                        None
                    };

                match negotiated {
                    Some(ct) => {
                        let mut hdrs = HashMap::new();
                        hdrs.insert("content-type".to_string(), ct.to_string());
                        let filename = if ct == "application/gzip" {
                            format!("{}.tar.gz", id)
                        } else {
                            format!("{}.zip", id)
                        };
                        hdrs.insert(
                            "content-disposition".to_string(),
                            format!("attachment; filename=\"{}\"", filename),
                        );

                        // Locate envelope directory
                        let package_dir = package_dir_for(&store_root, id);
                        if !package_dir.exists() {
                            set_response(
                                http::StatusCode::NOT_FOUND,
                                HashMap::new(),
                                Some("Envelope not found".into()),
                                None,
                                None,
                            );
                            // Prevent forwarding to backends
                            envelope
                                .request_details
                                .metadata
                                .insert("skip_backends".to_string(), "true".to_string());
                            return Ok(envelope);
                        }

                        // Build archive bytes according to negotiated content type
                        let bytes = if ct == "application/gzip" {
                            match make_targz(&package_dir) {
                                Ok(b) => b,
                                Err(e) => {
                                    set_response(
                                        http::StatusCode::INTERNAL_SERVER_ERROR,
                                        HashMap::new(),
                                        Some(format!("archive error: {}", e)),
                                        None,
                                        None,
                                    );
                                    // Prevent forwarding to backends
                                    envelope
                                        .request_details
                                        .metadata
                                        .insert("skip_backends".to_string(), "true".to_string());
                                    return Ok(envelope);
                                }
                            }
                        } else {
                            match make_zip(&package_dir) {
                                Ok(b) => b,
                                Err(e) => {
                                    set_response(
                                        http::StatusCode::INTERNAL_SERVER_ERROR,
                                        HashMap::new(),
                                        Some(format!("archive error: {}", e)),
                                        None,
                                        None,
                                    );
                                    // Prevent forwarding to backends
                                    envelope
                                        .request_details
                                        .metadata
                                        .insert("skip_backends".to_string(), "true".to_string());
                                    return Ok(envelope);
                                }
                            }
                        };

                        // Encode as base64 to emit safely through normalized_data
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        set_response(http::StatusCode::OK, hdrs, None, None, Some(b64));
                        // Prevent forwarding to backends for JMIX-served routes
                        envelope
                            .request_details
                            .metadata
                            .insert("skip_backends".to_string(), "true".to_string());
                    }
                    None => {
                        set_response(
                            http::StatusCode::NOT_ACCEPTABLE,
                            HashMap::new(),
                            Some(String::from("unsupported media type in Accept")),
                            None,
                            None,
                        );
                    }
                }
                return Ok(envelope);
            }
        }


        // GET/HEAD /api/jmix/{id}/manifest
        if (method == "GET" || method == "HEAD") && subpath.starts_with("api/jmix/") && subpath.ends_with("/manifest") {
            let rest = &subpath["api/jmix/".len()..];
            if let Some(id_part) = rest.strip_suffix("/manifest") {
                let id = id_part.trim_end_matches('/');
                if !id.is_empty() && !id.contains('/') {
                    let mut hdrs = HashMap::new();
                    hdrs.insert("content-type".to_string(), "application/json".to_string());

                    // Load manifest.json
                    let package_dir = package_dir_for(&store_root, id);
                    let manifest_path = package_dir.join("manifest.json");
                    if !manifest_path.exists() {
                        set_response(
                            http::StatusCode::NOT_FOUND,
                            HashMap::new(),
                            Some("manifest.json not found".into()),
                            None,
                            None,
                        );
                        return Ok(envelope);
                    }
                    match fs::read_to_string(&manifest_path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    {
                        Some(mut json) => {
                            // Ensure manifest has id; inject if missing
                            // Ensure the response JSON includes the envelope id
                            let has_id = json.get("id").and_then(|v| v.as_str()).is_some();
                            if !has_id {
                                if let Some(obj) = json.as_object_mut() {
                                    obj.insert("id".to_string(), serde_json::json!(id));
                                }
                            }
                            set_response(http::StatusCode::OK, hdrs, None, Some(json), None);
                            // Prevent forwarding to backends for JMIX-served routes
                            envelope
                                .request_details
                                .metadata
                                .insert("skip_backends".to_string(), "true".to_string());
                        }
                        None => {
                            set_response(
                                http::StatusCode::INTERNAL_SERVER_ERROR,
                                HashMap::new(),
                                Some("failed to parse manifest.json".into()),
                                None,
                                None,
                            );
                        }
                    }
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
            if let Some(uid) = study_uid.clone() {
                // Check local store first
                let matches = query_by_study_uid(&store_root, &uid)?;
                tracing::debug!("JMIX DEBUG: uid='{}', matches.len()={}", uid, matches.len());
                if !matches.is_empty() {
                    // Negotiate Accept. If JSON explicitly requested, return index JSON.
                    let accept_header = accept.unwrap_or("");
                    let wants_json = accept_header.contains("application/json");
                    let wants_gzip = accept_header.contains("application/gzip") || accept_header.contains("application/x-gtar");
                    let wants_zip = accept_header.contains("application/zip");
                    
                    tracing::debug!("JMIX DEBUG: accept_header='{}', wants_json={}, wants_gzip={}, wants_zip={}, matches.len()={}", accept_header, wants_json, wants_gzip, wants_zip, matches.len());

                    let condition = (wants_zip || wants_gzip || !wants_json) && matches.len() == 1;
                    tracing::debug!("JMIX DEBUG: condition result = {}", condition);
                    if condition {
                        // Return the package directly (zip default; gzip if explicitly requested)
                        let m = &matches[0];
                        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let path = m.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let package_dir = Path::new(path);
                        let ct = if wants_gzip { "application/gzip" } else { "application/zip" };
                        let mut hdrs = HashMap::new();
                        hdrs.insert("content-type".to_string(), ct.to_string());
                        let filename = if ct == "application/gzip" {
                            format!("{}.tar.gz", id)
                        } else {
                            format!("{}.zip", id)
                        };
                        hdrs.insert(
                            "content-disposition".to_string(),
                            format!("attachment; filename=\"{}\"", filename),
                        );

                        let bytes = if ct == "application/gzip" {
                            match make_targz(&package_dir) {
                                Ok(b) => b,
                                Err(e) => {
                                    set_response(
                                        http::StatusCode::INTERNAL_SERVER_ERROR,
                                        HashMap::new(),
                                        Some(format!("archive error: {}", e)),
                                        None,
                                        None,
                                    );
                                    envelope
                                        .request_details
                                        .metadata
                                        .insert("skip_backends".to_string(), "true".to_string());
                                    return Ok(envelope);
                                }
                            }
                        } else {
                            match make_zip(&package_dir) {
                                Ok(b) => b,
                                Err(e) => {
                                    set_response(
                                        http::StatusCode::INTERNAL_SERVER_ERROR,
                                        HashMap::new(),
                                        Some(format!("archive error: {}", e)),
                                        None,
                                        None,
                                    );
                                    envelope
                                        .request_details
                                        .metadata
                                        .insert("skip_backends".to_string(), "true".to_string());
                                    return Ok(envelope);
                                }
                            }
                        };
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        set_response(http::StatusCode::OK, hdrs, None, None, Some(b64));
                        envelope
                            .request_details
                            .metadata
                            .insert("skip_backends".to_string(), "true".to_string());
                        return Ok(envelope);
                    }

                    // Otherwise, return an index of envelopes as JSON (multiple matches or JSON requested)
                    let mut hdrs = HashMap::new();
                    hdrs.insert("content-type".to_string(), "application/json".to_string());
                    let jmix_envelopes: Vec<serde_json::Value> = matches
                        .into_iter()
                        .map(|m| {
                            let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let path = m.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            let suid = m
                                .get("studyInstanceUid")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            serde_json::json!({
                                "id": id,
                                "storePath": path,
                                "studyInstanceUid": suid
                            })
                        })
                        .collect();
                    let json = serde_json::json!({
                        "studyInstanceUid": uid,
                        "jmixEnvelopes": jmix_envelopes
                    });
                    set_response(http::StatusCode::OK, hdrs, None, Some(json), None);
                    envelope
                        .request_details
                        .metadata
                        .insert("skip_backends".to_string(), "true".to_string());
                    return Ok(envelope);
                }

                // No existing JMIX; trigger a DIMSE operation via backend by setting the identifier.
                // Orthanc and many PACS do not serve as C-GET SCPs by default; prefer C-MOVE here so the
                // PACS pushes instances to our local Store SCP. The jmix_builder middleware will then
                // package the received instances into a JMIX envelope.
                let identifier = serde_json::json!({
                    "0020000D": { "vr": "UI", "Value": [ uid ] }
                });
                envelope.original_data = serde_json::to_vec(&identifier)
                    .map_err(|e| Error::from(format!("identifier encode error: {}", e)))?;
                // Signal the DICOM backend to perform a C-MOVE (preferred for Orthanc and most PACS)
                envelope
                    .request_details
                    .metadata
                    .insert("dimse_op".to_string(), "move".to_string());
                // Also set path for compatibility, though dimse_op takes precedence
                envelope
                    .request_details
                    .metadata
                    .insert("path".to_string(), "move".to_string());
                // Do NOT set a response here; allow backends + jmix_builder to run and produce it
                return Ok(envelope);
            } else {
                set_response(
                    http::StatusCode::BAD_REQUEST,
                    HashMap::new(),
                    Some("missing studyInstanceUid".into()),
                    None,
                    None,
                );
                // This is fully served by JMIX
                envelope
                    .request_details
                    .metadata
                    .insert("skip_backends".to_string(), "true".to_string());
                return Ok(envelope);
            }
        }

        // POST /api/jmix (upload envelope)
        if method == "POST" && subpath == "api/jmix" {
            let content_type = headers
                .get("content-type")
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            let is_zip = content_type.contains("zip");
            let is_gzip = content_type.contains("gzip") || content_type.contains("x-gtar");

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

            // Extract archive
            let extracted_root = temp_extract_dir.path().to_path_buf();
            let body = &envelope.original_data;
            let extract_res = if is_zip {
                extract_zip(body, &extracted_root)
            } else if is_gzip {
                extract_targz(body, &extracted_root)
            } else {
                Err(Error::from("Unsupported Content-Type for JMIX upload"))
            };

            if let Err(e) = extract_res {
                set_response(
                    http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    HashMap::new(),
                    Some(format!("extract error: {}", e)),
                    None,
                    None,
                );
                return Ok(envelope);
            }

            // Find manifest.json in extracted content
            let (pkg_dir, manifest_json) = find_package_root_and_manifest(&extracted_root)?;
            let id = manifest_json
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if id.is_empty() {
                set_response(
                    http::StatusCode::BAD_REQUEST,
                    HashMap::new(),
                    Some("manifest.id missing".into()),
                    None,
                    None,
                );
                return Ok(envelope);
            }

            // Move into store_root/{id}.jmix
            let dest_dir = package_dir_for(&store_root, id);
            if dest_dir.exists() {
                set_response(
                    http::StatusCode::CONFLICT,
                    HashMap::new(),
                    Some("envelope id already exists".into()),
                    None,
                    None,
                );
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
                    let mut hdrs = HashMap::new();
                    hdrs.insert("content-type".to_string(), "application/json".to_string());
                    let json = serde_json::json!({ "id": id, "status": "stored" });
                    set_response(http::StatusCode::CREATED, hdrs, None, Some(json), None);
                }
                Err(e) => {
                    // Cleanup on failure
                    let _ = fs::remove_dir_all(&dest_dir);
                    set_response(
                        http::StatusCode::BAD_REQUEST,
                        HashMap::new(),
                        Some(format!("validation failed: {}", e)),
                        None,
                        None,
                    );
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
        set_response(
            http::StatusCode::NOT_FOUND,
            HashMap::new(),
            Some(String::from("Not Found")),
            None,
            None,
        );
        Ok(envelope)
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

        // body as base64 (binary)
        if let Some(body_b64) = response_meta
            .and_then(|m| m.get("body_b64"))
            .and_then(|b| b.as_str())
        {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(body_b64)
                .map_err(|_| Error::from("Failed to decode body_b64"))?;
            return builder
                .body(Body::from(bytes))
                .map_err(|_| Error::from("Failed to construct JMIX HTTP response"));
        }

        // body as explicit string
        if let Some(body_str) = response_meta
            .and_then(|m| m.get("body"))
            .and_then(|b| b.as_str())
        {
            return builder
                .body(Body::from(body_str.to_string()))
                .map_err(|_| Error::from("Failed to construct JMIX HTTP response"));
        }

        // body as JSON object under response.json
        if let Some(json_val) = response_meta.and_then(|m| m.get("json")) {
            let body_str = serde_json::to_string(json_val)
                .map_err(|_| Error::from("Failed to serialize JMIX response JSON"))?;
            if !has_content_type {
                builder = builder.header("content-type", "application/json");
            }
            return builder
                .body(Body::from(body_str))
                .map_err(|_| Error::from("Failed to construct JMIX HTTP response"));
        }

        // default: serialize entire normalized_data for debug
        let body_str = serde_json::to_string(&nd)
            .map_err(|_| Error::from("Failed to serialize Jmix response payload into JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct JMIX HTTP response"))
    }
}
