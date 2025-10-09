use crate::globals::get_storage;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{PathBuf};

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
        let _folder_id = nd
            .get("folder_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let _file_count = nd
            .get("file_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let instances = nd.get("instances").cloned().unwrap_or(serde_json::json!([]));

        let is_dicom_result = (operation == "move" || operation == "get") && success && folder_path.is_some();

        if !is_dicom_result {
            return Ok(envelope);
        }

        let folder_path = match folder_path {
            Some(p) => p,
            None => return Ok(envelope),
        };

        // Create JMIX package using jmix-rs builder (manifest.json, metadata.json, files.json)
        let (store_root, pkg_root, _payload_dir) = ensure_jmix_package_dirs().map_err(Error::from)?;

        // Prepare a minimal JMIX config. We don't enable validation/signing here.
        let mut jcfg = jmix_rs::config::Config::default();
        jcfg.sender.name = "Harmony Proxy".to_string();
        jcfg.sender.id = "org:harmony-proxy".to_string();
        jcfg.requester.name = "Harmony Proxy".to_string();
        jcfg.requester.id = "org:harmony-proxy".to_string();

        // Build envelope from the DICOM folder path provided by the DIMSE backend
        let builder = jmix_rs::builder::JmixBuilder::new();
        let (envelope_built, dicom_files) = builder
            .build_from_dicom(&folder_path, &jcfg)
            .map_err(|e| Error::from(format!("jmix build error: {}", e)))?;

        // Persist envelope to the JMIX store root
        let _saved = builder
            .save_to_files(&envelope_built, &dicom_files, &pkg_root)
            .map_err(|e| Error::from(format!("jmix save error: {}", e)))?;

        // Prepare response JSON with the created envelope id and path
        let jmix_id = envelope_built.manifest.id.clone();
        let pkg_dir = store_root.join(format!("{}.jmix", jmix_id));
        let study_uid = envelope_built
            .metadata
            .studies
            .as_ref()
            .and_then(|s| s.study_uid.clone())
            .unwrap_or_else(|| extract_study_uid(&instances));

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
        // Mirror response into both normalized_data and original_data so it is preserved
        // by the pipeline and directly accessible in unit tests.
        let out_nd = new_nd;
        envelope.normalized_data = Some(out_nd.clone());
        envelope.original_data = out_nd;
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
    use std::fs;

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
