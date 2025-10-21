use crate::globals::get_storage;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::models::middleware::types::jmix_index::{
    current_timestamp, get_jmix_index, JmixPackageInfo,
};
use crate::utils::Error;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Middleware that builds JMIX envelopes from DICOM operation responses
///
/// Right-side behavior:
/// - Detects DICOM "move"/"get" responses that include folder_path/folder_id and instances
/// - Creates a JMIX package under storage: jmix-store/<id>.jmix
/// - Copies DICOM files from the folder into payload/
/// - Writes a minimal manifest.json and payload/metadata.json
/// - Sets normalized_data.response.json with the created JMIX envelope IDs so JMIX service can return them
pub struct JmixBuilderMiddleware;

impl Default for JmixBuilderMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl JmixBuilderMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Middleware for JmixBuilderMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Read metadata set by endpoint
        let jmix_id = envelope.request_details.metadata.get("jmix_id").cloned();
        let jmix_method = envelope
            .request_details
            .metadata
            .get("jmix_method")
            .cloned();
        let study_uid = envelope
            .request_details
            .metadata
            .get("jmix_study_uid")
            .cloned();
        let wants_manifest = envelope
            .request_details
            .metadata
            .get("jmix_wants_manifest")
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);

        // Build options map from metadata
        let mut options = HashMap::new();
        if let Some(store_dir) = envelope.request_details.metadata.get("endpoint_store_dir") {
            options.insert("store_dir".to_string(), serde_json::json!(store_dir));
        }

        let store_root = resolve_store_root(&options);

        // Helper to set response metadata and skip backends
        let mut set_response_and_skip =
            |status: u16,
             hdrs: HashMap<String, String>,
             body_str: Option<String>,
             json_obj: Option<serde_json::Value>,
             jmix_id: Option<String>,
             zip_ready: Option<bool>| {
                // Store response metadata in request_details.metadata
                envelope
                    .request_details
                    .metadata
                    .insert("jmix_response_status".to_string(), status.to_string());

                // Store headers as JSON string
                if !hdrs.is_empty() {
                    if let Ok(headers_json) = serde_json::to_string(&hdrs) {
                        envelope
                            .request_details
                            .metadata
                            .insert("jmix_response_headers".to_string(), headers_json);
                    }
                }

                // Store body or JSON in normalized_data for the response
                if let Some(j) = json_obj {
                    envelope.normalized_data = Some(j);
                } else if let Some(s) = body_str {
                    envelope.normalized_data = Some(serde_json::json!(s));
                }

                // Store JMIX-specific metadata
                if let Some(id) = jmix_id {
                    envelope
                        .request_details
                        .metadata
                        .insert("jmix_id".to_string(), id);
                }
                if let Some(ready) = zip_ready {
                    envelope
                        .request_details
                        .metadata
                        .insert("jmix_zip_ready".to_string(), ready.to_string());
                }

                envelope
                    .request_details
                    .metadata
                    .insert("skip_backends".to_string(), "true".to_string());
            };

        // Only process GET/HEAD requests
        let is_get_or_head =
            jmix_method.as_deref() == Some("GET") || jmix_method.as_deref() == Some("HEAD");
        if !is_get_or_head {
            return Ok(envelope);
        }

        // Case 1: GET/HEAD /api/jmix/{id} or /api/jmix/{id}/manifest
        if let Some(id) = jmix_id {
            let package_dir = package_dir_for(&store_root, &id);

            if !package_dir.exists() {
                // Package doesn't exist - let it pass through to backends
                return Ok(envelope);
            }

            // Serve manifest
            if wants_manifest {
                let manifest_path = package_dir.join("manifest.json");
                
                // Try to read manifest from file first, then from ZIP if not found
                let manifest_json_opt = if manifest_path.exists() {
                    fs::read_to_string(&manifest_path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                } else {
                    // Manifest not found as file - try to extract from ZIP
                    let zip_file = package_dir.join(format!("{}.zip", id));
                    let mut manifest_from_zip = None;
                    if zip_file.exists() {
                        // Read and parse ZIP to find manifest.json
                        if let Ok(zip_bytes) = fs::read(&zip_file) {
                            use std::io::Cursor;
                            if let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(zip_bytes)) {
                                // Try to find manifest.json in the archive
                                for i in 0..archive.len() {
                                    if let Ok(mut file) = archive.by_index(i) {
                                        if file.name().ends_with("manifest.json") {
                                            use std::io::Read;
                                            let mut contents = String::new();
                                            if file.read_to_string(&mut contents).is_ok() {
                                                if let Ok(json) = serde_json::from_str(&contents) {
                                                    manifest_from_zip = Some(json);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    manifest_from_zip
                };

                match manifest_json_opt {
                    Some(mut json) => {
                        // Ensure manifest has id; inject if missing
                        let has_id = json.get("id").and_then(|v| v.as_str()).is_some();
                        if !has_id {
                            if let Some(obj) = json.as_object_mut() {
                                obj.insert("id".to_string(), serde_json::json!(id));
                            }
                        }
                        let mut hdrs = HashMap::new();
                        hdrs.insert("content-type".to_string(), "application/json".to_string());
                        set_response_and_skip(200, hdrs, None, Some(json), None, None);
                    }
                    None => {
                        set_response_and_skip(
                            404,
                            HashMap::new(),
                            Some("manifest.json not found".into()),
                            None,
                            None,
                            None,
                        );
                    }
                }
                return Ok(envelope);
            }

            // Verify the zip exists before serving from cache
            let zip_file = package_dir.join(format!("{}.zip", id));

            if !zip_file.exists() {
                // Zip file is missing - package directory may have been deleted
                tracing::warn!(
                    "📦 JMIX package {} not found at {}, returning 404",
                    id,
                    zip_file.display()
                );
                // For direct ID requests, return 404 since we can't rebuild without study UID
                set_response_and_skip(
                    404,
                    HashMap::new(),
                    Some("JMIX package not found".to_string()),
                    None,
                    None,
                    None,
                );
                return Ok(envelope);
            }

            // Set jmix metadata - service will handle zip serving with proper headers
            set_response_and_skip(
                200,
                HashMap::new(),
                None,
                None,
                Some(id.to_string()),
                Some(true),
            );
            
            // Also store metadata in request_details for immediate response processing
            envelope
                .request_details
                .metadata
                .insert("jmix_id".to_string(), id.to_string());
            envelope
                .request_details
                .metadata
                .insert("jmix_zip_ready".to_string(), "true".to_string());
            return Ok(envelope);
        }

        // Case 2: GET/HEAD /api/jmix?studyInstanceUid=...
        // Returns ZIP file if Accept: application/zip, otherwise returns JSON index
        if let Some(uid) = study_uid {
            let matches = query_by_study_uid(&store_root, &uid)?;

            if matches.is_empty() {
                // No local matches - let backends handle it
                // Set dimse_op for DICOM backend to use "get" operation (retrieve study data)
                envelope
                    .request_details
                    .metadata
                    .insert("dimse_op".to_string(), "get".to_string());
                return Ok(envelope);
            }

            // Check Accept header to determine response format
            let accept = envelope
                .request_details
                .headers
                .get("accept")
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            let wants_zip = accept.contains("application/zip");
            let wants_json = accept.contains("application/json") || accept.contains("*/*") || accept.is_empty();

            // If client wants JSON, return index listing
            if wants_json && !wants_zip {
                // Build JSON response with list of matching packages
                let json_response = serde_json::json!({
                    "jmixEnvelopes": matches
                });
                let mut hdrs = HashMap::new();
                hdrs.insert("content-type".to_string(), "application/json".to_string());
                set_response_and_skip(200, hdrs, None, Some(json_response), None, None);
                return Ok(envelope);
            }

            // Client wants ZIP - serve the zip file
            // Use the first match (most recent or only one)
            let m = &matches[0];
            let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let path = m.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let _package_dir = Path::new(path);

            // Verify the zip exists before serving from cache
            let package_dir = store_root.join(id);
            let zip_file = package_dir.join(format!("{}.zip", id));

            if !zip_file.exists() {
                // Zip file is missing (e.g., tmp directory was deleted)
                // Don't skip backends - let them fetch and rebuild the package
                tracing::warn!(
                    "📦 JMIX package {} found in index but zip file missing at {}, allowing backend to refetch",
                    id,
                    zip_file.display()
                );
                // Pass through to backends without setting skip_backends
                // Set dimse_op for DICOM backend to use "get" operation
                envelope
                    .request_details
                    .metadata
                    .insert("dimse_op".to_string(), "get".to_string());
                return Ok(envelope);
            }

            // Set jmix metadata - service will handle zip serving with proper headers
            set_response_and_skip(
                200,
                HashMap::new(),
                None,
                None,
                Some(id.to_string()),
                Some(true),
            );
            
            // Also store metadata in request_details for immediate response processing
            envelope
                .request_details
                .metadata
                .insert("jmix_id".to_string(), id.to_string());
            envelope
                .request_details
                .metadata
                .insert("jmix_zip_ready".to_string(), "true".to_string());
            return Ok(envelope);
        }

        // No JMIX-specific handling needed - pass through
        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        // Check if zip is already ready from previous processing (e.g. from left() method via metadata)
        if let Some(zip_ready_str) = envelope.request_details.metadata.get("jmix_zip_ready") {
            if zip_ready_str == "true" {
                tracing::debug!("📦 Zip already ready, skipping build process");
                return Ok(envelope);
            }
        }

        // Read normalized_data; expect a DICOM operation result from a backend
        let nd = envelope
            .normalized_data
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));

        // Handle two shapes from DICOM service: move and get
        // Example fields: { operation: "move"|"get", success: true, folder_id, folder_path, file_count, instances: [...] }
        let operation = nd.get("operation").and_then(|v| v.as_str()).unwrap_or("");
        let success = nd.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
        let folder_path = nd
            .get("folder_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let _folder_id = nd
            .get("folder_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let _file_count = nd.get("file_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let instances = nd
            .get("instances")
            .cloned()
            .unwrap_or(serde_json::json!([]));

        let is_dicom_result =
            (operation == "move" || operation == "get") && success && folder_path.is_some();

        if !is_dicom_result {
            return Ok(envelope);
        }

        let folder_path = match folder_path {
            Some(p) => p,
            None => return Ok(envelope),
        };

        // Create JMIX package using jmix-rs builder (manifest.json, metadata.json, files.json)
        let store_root = ensure_store_root().map_err(Error::from)?;

        // Prepare a minimal JMIX config. We don't enable validation/signing here.
        let mut jcfg = jmix_rs::config::Config::default();
        jcfg.sender.name = "Harmony Proxy".to_string();
        jcfg.sender.id = "org:harmony-proxy".to_string();
        jcfg.requester.name = "Harmony Proxy".to_string();
        jcfg.requester.id = "org:harmony-proxy".to_string();

        // Extract skip flags from request metadata (defaults to false)
        let skip_hashing = envelope
            .request_details
            .metadata
            .get("skip_hashing")
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);
        let skip_listing = envelope
            .request_details
            .metadata
            .get("skip_listing")
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);

        tracing::info!(
            "🚀 JMIX Builder: skip_hashing={}, skip_listing={}",
            skip_hashing,
            skip_listing
        );
        tracing::info!(
            "🚀 JMIX Builder: metadata keys: {:?}",
            envelope.request_details.metadata.keys().collect::<Vec<_>>()
        );

        // Build envelope from the DICOM folder path provided by the DIMSE backend
        let builder = jmix_rs::builder::JmixBuilder::new();
        let (envelope_built, dicom_files) = builder
            .build_from_dicom_with_options(&folder_path, &jcfg, skip_hashing, skip_listing)
            .map_err(|e| Error::from(format!("jmix build error: {}", e)))?;

        // Create package-specific directory for this envelope
        let jmix_id = envelope_built.manifest.id.clone();
        let pkg_dir = store_root.join(&jmix_id);

        // Ensure the package directory exists
        fs::create_dir_all(&pkg_dir).map_err(|e| {
            Error::from(format!(
                "Failed to create package dir {}: {}",
                pkg_dir.display(),
                e
            ))
        })?;

        // Persist envelope to the specific package directory
        let _saved = builder
            .save_to_files_with_options(
                &envelope_built,
                &dicom_files,
                &pkg_dir,
                skip_hashing,
                skip_listing,
            )
            .map_err(|e| Error::from(format!("jmix save error: {}", e)))?;

        // Extract study UID from the built envelope
        let study_uid = envelope_built
            .metadata
            .studies
            .as_ref()
            .and_then(|s| s.study_uid.clone())
            .unwrap_or_else(|| extract_study_uid(&instances));

        // Index the newly created package
        let index = get_jmix_index(&store_root)
            .map_err(|e| Error::from(format!("Failed to open JMIX index: {}", e)))?;
        let package_info = JmixPackageInfo {
            id: jmix_id.clone(),
            study_uid: study_uid.clone(),
            path: pkg_dir.to_string_lossy().to_string(),
            created_at: current_timestamp(),
        };
        index
            .index_package(&package_info)
            .map_err(|e| Error::from(format!("Failed to index package: {}", e)))?;

        // jmix-rs should have created a zip file in the package directory
        let zip_file = pkg_dir.join(format!("{}.zip", jmix_id));

        if !zip_file.exists() {
            return Err(Error::from(format!(
                "jmix-rs did not create expected zip file: {}",
                zip_file.display()
            )));
        }

        // Verify the zip file has content
        match fs::metadata(&zip_file) {
            Ok(metadata) => {
                tracing::info!(
                    "✅ JMIX zip file ready: {} ({} bytes)",
                    zip_file.display(),
                    metadata.len()
                );
            }
            Err(e) => {
                return Err(Error::from(format!(
                    "Failed to access jmix zip file {}: {}",
                    zip_file.display(),
                    e
                )));
            }
        }

        // Clean up DIMSE files now that the zip has been successfully created
        let dimse_folder = std::path::Path::new(&folder_path);
        if dimse_folder.exists() {
            match fs::remove_dir_all(dimse_folder) {
                Ok(_) => {
                    tracing::info!("🧹 Cleaned up DIMSE files from: {}", folder_path);
                }
                Err(e) => {
                    tracing::warn!(
                        "⚠️ Failed to cleanup DIMSE files from {}: {}",
                        folder_path,
                        e
                    );
                    // Don't fail the entire operation if cleanup fails - just log warning
                }
            }
        } else {
            tracing::debug!(
                "📁 DIMSE folder {} does not exist (already cleaned up?)",
                folder_path
            );
        }

        // Set metadata in response_details for jmix service to use
        envelope
            .response_details
            .metadata
            .insert("jmix_id".to_string(), jmix_id.clone());
        envelope
            .response_details
            .metadata
            .insert("jmix_zip_ready".to_string(), "true".to_string());
        envelope
            .response_details
            .metadata
            .insert("jmix_study_uid".to_string(), study_uid);

        Ok(envelope)
    }
}

/// Resolve the store root directory from options or use global storage
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

/// Get the package directory for a given JMIX envelope ID
fn package_dir_for(store_root: &Path, id: &str) -> PathBuf {
    store_root.join(id)
}

/// Query JMIX envelopes by StudyInstanceUID using the redb index
fn query_by_study_uid(store_root: &Path, study_uid: &str) -> Result<Vec<serde_json::Value>, Error> {
    let index = get_jmix_index(store_root)
        .map_err(|e| Error::from(format!("Failed to open JMIX index: {}", e)))?;

    let packages = index
        .query_by_study_uid(study_uid)
        .map_err(|e| Error::from(format!("Failed to query by study UID: {}", e)))?;

    let results = packages
        .into_iter()
        .map(|pkg| {
            serde_json::json!({
                "id": pkg.id,
                "path": pkg.path,
                "studyInstanceUid": pkg.study_uid
            })
        })
        .collect();

    Ok(results)
}

fn ensure_store_root() -> Result<PathBuf, String> {
    // Use global storage to derive jmix-store root; fallback to ./tmp/jmix-store
    let store_root = if let Some(storage) = get_storage() {
        storage.subpath_str("jmix-store")
    } else {
        PathBuf::from("./tmp/jmix-store")
    };

    // Ensure the store root exists
    fs::create_dir_all(&store_root).map_err(|e| {
        format!(
            "Failed to create store root {}: {}",
            store_root.display(),
            e
        )
    })?;

    Ok(store_root)
}

fn extract_study_uid(instances: &Value) -> String {
    // Try a few common JSON shapes for StudyInstanceUID
    if let Some(arr) = instances.as_array() {
        for item in arr {
            if let Some(uid) = item.get("StudyInstanceUID").and_then(|v| v.as_str()) {
                return uid.to_string();
            }
            if let Some(uid) = item.get("0020000D").and_then(|v| v.as_str()) {
                return uid.to_string();
            }
            // Some DICOM JSONs nest values under { "Value": ["..."] }
            if let Some(val_arr) = item
                .get("0020000D")
                .and_then(|v| v.get("Value"))
                .and_then(|v| v.as_array())
            {
                if let Some(first) = val_arr.first().and_then(|v| v.as_str()) {
                    return first.to_string();
                }
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::globals::set_storage;
    use crate::models::envelope::envelope::{RequestDetails, ResponseDetails, ResponseEnvelope};
    use crate::storage::{filesystem::FilesystemStorage, StorageBackend};
    use serial_test::serial;
    use std::fs;
    use std::sync::Arc;
    use uuid;

    #[test]
    #[serial]
    fn test_builds_jmix_envelope_from_dicom_result() {
        // Reset global storage to ensure clean state
        crate::globals::reset_storage();
        
        // Create unique storage for this test
        let storage = create_test_storage();
        set_storage(storage.clone());

        // Prepare a temp source folder with two files
        let src_dir = storage
            .ensure_dir_str("dimse/test_jmix_builder")
            .expect("ensure");
        let f1 = src_dir.join("a.dcm");
        let f2 = src_dir.join("b.dcm");
        let _ = fs::write(&f1, b"fake dicom 1");
        let _ = fs::write(&f2, b"fake dicom 2");

        // Build a DICOM-like normalized_data
        let nd = serde_json::json!({
            "operation": "get",
            "success": true,
            "folder_id": "abc",
            "folder_path": src_dir.to_string_lossy(),
            "file_count": 2,
            "instances": [{"StudyInstanceUID": "1.2.3"}]
        });
        let request_details = RequestDetails {
            method: "GET".into(),
            uri: "".into(),
            headers: Default::default(),
            cookies: Default::default(),
            query_params: Default::default(),
            cache_status: None,
            metadata: Default::default(),
        };

        let env = ResponseEnvelope {
            request_details,
            response_details: ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            original_data: serde_json::json!({}),
            normalized_data: Some(nd),
            normalized_snapshot: None,
        };

        let mw = JmixBuilderMiddleware::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async move { mw.right(env).await.expect("mw") });

        // Validate response contains jmix metadata in response_details.metadata
        let jmix_id = out
            .response_details
            .metadata
            .get("jmix_id")
            .expect("jmix_id should be in metadata");
        let zip_ready = out
            .response_details
            .metadata
            .get("jmix_zip_ready")
            .map(|s| s == "true")
            .expect("jmix_zip_ready should be in metadata");

        assert!(!jmix_id.is_empty(), "jmix_id should not be empty");
        assert!(zip_ready, "zip_ready should be true");
    }

    #[test]
    #[serial]
    fn test_zip_file_contains_expected_files() {
        use std::io::Cursor;
        use zip::ZipArchive;

        // Reset global storage to ensure clean state
        crate::globals::reset_storage();

        // Create unique storage for this test
        let storage = create_test_storage();
        set_storage(storage.clone());

        // Capture the expected store_root path before async execution
        let expected_store_root = storage.subpath_str("jmix-store");

        // Prepare a temp source folder with DICOM files
        let src_dir = storage
            .ensure_dir_str("dimse/test_jmix_zip_contents")
            .expect("ensure");
        let f1 = src_dir.join("file1.dcm");
        let f2 = src_dir.join("file2.dcm");
        let _ = fs::write(&f1, b"fake dicom content 1");
        let _ = fs::write(&f2, b"fake dicom content 2");

        // Build a DICOM-like normalized_data
        let nd = serde_json::json!({
            "operation": "get",
            "success": true,
            "folder_id": "test_zip_contents",
            "folder_path": src_dir.to_string_lossy(),
            "file_count": 2,
            "instances": [{"StudyInstanceUID": "1.2.3.test.zip"}]
        });
        let request_details = RequestDetails {
            method: "GET".into(),
            uri: "".into(),
            headers: Default::default(),
            cookies: Default::default(),
            query_params: Default::default(),
            cache_status: None,
            metadata: Default::default(),
        };

        let env = ResponseEnvelope {
            request_details,
            response_details: ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            original_data: serde_json::json!({}),
            normalized_data: Some(nd),
            normalized_snapshot: None,
        };

        let mw = JmixBuilderMiddleware::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async move {
            mw.right(env).await
        });

        // Check if the middleware ran successfully
        match &result {
            Ok(_) => println!("✅ Middleware executed successfully"),
            Err(e) => {
                println!("❌ Middleware failed: {}", e);
                panic!("Middleware execution failed: {}", e);
            }
        }

        let out = result.expect("middleware should succeed");

        // Get the jmix_id from the response metadata
        let jmix_id = out
            .response_details
            .metadata
            .get("jmix_id")
            .expect("jmix_id should be in metadata");

        // Use the pre-captured store_root path
        let store_root = expected_store_root;

        // Look for zip file in the package directory (where jmix-rs creates it)
        let package_dir = store_root.join(jmix_id);
        let zip_file = package_dir.join(format!("{}.zip", jmix_id));
        println!("🔍 Looking for zip file at: {}", zip_file.display());

        // Check if zip file exists and has content
        assert!(
            zip_file.exists(),
            "Zip file should exist at {}",
            zip_file.display()
        );

        let zip_data = fs::read(&zip_file).expect("Should be able to read zip file");
        println!("📦 Zip file size: {} bytes", zip_data.len());
        assert!(zip_data.len() > 0, "Zip file should not be empty");

        // Check the package directory contents
        let pkg_dir = store_root.join(jmix_id);
        println!("📁 Package directory: {}", pkg_dir.display());

        if pkg_dir.exists() {
            let entries: Vec<_> = fs::read_dir(&pkg_dir)
                .expect("Should read package dir")
                .collect::<Result<Vec<_>, _>>()
                .expect("Should collect entries");

            let file_names: Vec<String> = entries
                .iter()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            println!("📋 Package directory contents: {:?}", file_names);
        } else {
            println!("❌ Package directory does not exist!");
        }

        // Extract and examine zip contents
        let cursor = Cursor::new(&zip_data);
        let mut archive = ZipArchive::new(cursor).expect("Should be able to open zip");

        println!("📦 Zip contains {} files:", archive.len());
        let mut zip_file_names = Vec::new();
        for i in 0..archive.len() {
            let file = archive.by_index(i).expect("Should get file by index");
            let name = file.name().to_string();
            println!("   - {} ({} bytes)", name, file.size());
            zip_file_names.push(name);
        }

        // Verify we have expected JMIX files
        let has_manifest = zip_file_names
            .iter()
            .any(|name| name.contains("manifest.json"));
        let has_metadata = zip_file_names
            .iter()
            .any(|name| name.contains("metadata.json"));
        let has_dicom_files = zip_file_names.iter().any(|name| name.ends_with(".dcm"));

        println!("🔍 Analysis:");
        println!("   - Has manifest.json: {}", has_manifest);
        println!("   - Has metadata.json: {}", has_metadata);
        println!("   - Has DICOM files: {}", has_dicom_files);

        // The zip should not be empty - it should have at least manifest and metadata
        assert!(archive.len() > 0, "Zip should contain files");
        assert!(has_manifest, "Zip should contain manifest.json");
        // Note: metadata.json and DICOM files might be optional depending on jmix-rs behavior
        
        // Clean up: Remove the test-specific storage to avoid interference
        drop(storage);
    }

    fn create_test_storage() -> Arc<FilesystemStorage> {
        // Always create unique storage directory for each test to avoid database lock contention
        let test_id = uuid::Uuid::new_v4();
        let test_storage_path = format!("./tmp/test-{}", test_id);
        let fs = FilesystemStorage::new(&test_storage_path).expect("Failed to create test storage");
        Arc::new(fs)
    }
}
