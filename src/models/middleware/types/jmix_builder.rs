use crate::globals::get_storage;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // No-op on left side
        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
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
        let folder_id = nd
            .get("folder_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let file_count = nd
            .get("file_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let instances = nd.get("instances").cloned().unwrap_or(serde_json::json!([]));

        let is_dicom_result = (operation == "move" || operation == "get") && success && file_count > 0;

        if !is_dicom_result {
            return Ok(envelope);
        }

        let folder_path = match folder_path {
            Some(p) => p,
            None => return Ok(envelope),
        };
        let folder_id = folder_id.unwrap_or_else(|| "unknown".to_string());

        // Create JMIX package in storage
        let (_store_root, pkg_root, _payload_dir) = ensure_jmix_package_dirs().map_err(Error::from)?;
        let jmix_id = Uuid::new_v4().to_string();
        let pkg_dir = pkg_root.join(format!("{}.jmix", jmix_id));
        let payload_dir = pkg_dir.join("payload");
        fs::create_dir_all(&payload_dir).map_err(|e| Error::from(format!("mk payload: {}", e)))?;

        // Copy DICOM files into payload/
        let src = Path::new(&folder_path);
        if src.is_dir() {
            for e in fs::read_dir(src)
                .map_err(|e| Error::from(format!("readdir: {}", e)))?
                .flatten()
            {
                let p = e.path();
                if p.is_file() {
                    let name = p.file_name().unwrap_or_default();
                    let dest = payload_dir.join(name);
                    let _ = fs::copy(&p, &dest);
                }
            }
        }

        // Build minimal manifest.json
        let manifest = serde_json::json!({
            "id": jmix_id,
            "type": "envelope",
            "version": 1,
            "content": {"type": "directory", "path": "payload"}
        });
        let manifest_path = pkg_dir.join("manifest.json");
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap())
            .map_err(|e| Error::from(format!("write manifest: {}", e)))?;

        // Build payload/metadata.json with minimal fields (attempt to extract StudyInstanceUID from instances)
        let study_uid = extract_study_uid(&instances);
        let metadata = serde_json::json!({
            "id": jmix_id,
            "source": "dimse",
            "folder_id": folder_id,
            "file_count": file_count,
            "studies": { "study_uid": study_uid }
        });
        let md_path = payload_dir.join("metadata.json");
        fs::write(&md_path, serde_json::to_vec_pretty(&metadata).unwrap())
            .map_err(|e| Error::from(format!("write metadata: {}", e)))?;

        // Set response so JMIX service can return the created IDs
        let mut out_headers = HashMap::new();
        out_headers.insert("content-type".to_string(), "application/json".to_string());
        let response_json = serde_json::json!({
            "jmixEnvelopes": [
                { "id": jmix_id, "storePath": pkg_dir.to_string_lossy(), "studyInstanceUid": study_uid }
            ]
        });

        // Merge into normalized_data.response with body string for DICOM service compatibility
        let mut new_nd = nd;
        let body_str = serde_json::to_string(&response_json)
            .map_err(|e| Error::from(format!("serialize response: {}", e)))?;
        let response_obj = serde_json::json!({
            "status": 200u16,
            "headers": out_headers,
            "body": body_str
        });
        if let Some(map) = new_nd.as_object_mut() {
            map.insert("response".to_string(), response_obj);
        }
        // On the right chain, the pipeline copies original_data into the final envelope.normalized_data.
        // Therefore, write our response into original_data to be preserved.
        envelope.original_data = new_nd;
        Ok(envelope)
    }
}


fn ensure_jmix_package_dirs() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    // Use global storage to derive jmix-store root; fallback to ./tmp/jmix-store
    let store_root = if let Some(storage) = get_storage() {
        storage.subpath_str("jmix-store")
    } else {
        PathBuf::from("./tmp/jmix-store")
    };
    let pkg_root = store_root.clone();
    let payload_placeholder = pkg_root.join("__payload__");
    Ok((store_root, pkg_root, payload_placeholder))
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
            if let Some(val_arr) = item.get("0020000D").and_then(|v| v.get("Value")).and_then(|v| v.as_array()) {
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
    use crate::storage::filesystem::FilesystemStorage;
    use crate::models::envelope::envelope::{RequestDetails, RequestEnvelope};
    use std::sync::Arc;

    #[test]
    fn test_builds_jmix_envelope_from_dicom_result() {
        // Ensure storage is set
        let _ = set_test_storage();

        // Prepare a temp source folder with two files
        let storage = get_storage().expect("storage");
        let src_dir = storage.ensure_dir_str("dimse/test_jmix_builder").expect("ensure");
        let f1 = src_dir.join("a.dcm");
        let f2 = src_dir.join("b.dcm");
        let _ = fs::write(&f1, b"fake");
        let _ = fs::write(&f2, b"fake");

        // Build a DICOM-like normalized_data
        let nd = serde_json::json!({
            "operation": "get",
            "success": true,
            "folder_id": "abc",
            "folder_path": src_dir.to_string_lossy(),
            "file_count": 2,
            "instances": [{"StudyInstanceUID": "1.2.3"}]
        });

        let env = RequestEnvelope {
            request_details: RequestDetails{
                method: "GET".into(), uri: "".into(), headers: Default::default(), cookies: Default::default(), query_params: Default::default(), cache_status: None, metadata: Default::default()
            },
            original_data: serde_json::json!({}),
            normalized_data: Some(nd),
        };

        let mw = JmixBuilderMiddleware::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async move { mw.right(env).await.expect("mw") });

        // Validate response contains jmixEnvelopes
        let nd2 = out.normalized_data.expect("nd");
        let body = nd2.get("response").and_then(|r| r.get("body")).and_then(|b| b.as_str()).expect("body str");
        let json: serde_json::Value = serde_json::from_str(body).expect("json");
        assert!(json.get("jmixEnvelopes").and_then(|v| v.as_array()).map(|a| !a.is_empty()).unwrap_or(false));
    }

    fn set_test_storage() -> Result<(), String> {
        if get_storage().is_some() { return Ok(()); }
        let fs = FilesystemStorage::with_default_path().map_err(|e| format!("storage: {:?}", e))?;
        set_storage(Arc::new(fs));
        Ok(())
    }
}
