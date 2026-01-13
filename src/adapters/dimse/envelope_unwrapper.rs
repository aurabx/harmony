//! Envelope Unwrapper for DICOM JSON extraction
//!
//! This module handles the extraction of clean DICOM JSON objects from various
//! envelope/wrapper formats that may be present in pipeline responses.
//!
//! # Supported Formats
//!
//! - **Raw DICOM JSON**: Objects with 8-character hex tag keys (e.g., "00100010")
//! - **DIMSE backend envelope**: `{ "operation": "...", "success": true, "matches": [...] }`
//! - **DICOMweb middleware envelope**: `{ "dicomweb_response_type": "...", "dicomweb_data": [...] }`
//! - **Wrapper struct**: `{ "identifier": {...}, "command": {...} }`
//! - **Binary DICOM wrapper**: `{ "body_b64": "..." }` (base64-encoded Part10 file)

use serde_json::Value;

/// Result of unwrapping an envelope - either DICOM JSON objects, binary data, or file paths
#[derive(Debug)]
pub enum UnwrappedContent {
    /// One or more DICOM JSON objects ready for conversion
    DicomJson(Vec<Value>),
    /// Binary DICOM Part10 data (already decoded from base64)
    Binary(Vec<u8>),
    /// Path to a folder containing DICOM files (for C-GET/C-MOVE results)
    FolderPath(std::path::PathBuf),
    /// No content (empty response)
    Empty,
}

/// Extracts DICOM content from a response envelope's JSON data.
///
/// This function handles the various wrapper formats that may be present
/// and returns clean DICOM JSON objects or binary data.
pub fn unwrap_response(json_data: &Value) -> UnwrappedContent {
    // Handle null or empty
    if json_data.is_null() {
        return UnwrappedContent::Empty;
    }
    if let Some(obj) = json_data.as_object() {
        if obj.is_empty() {
            return UnwrappedContent::Empty;
        }
    }

    // Check for folder_path (C-GET/C-MOVE results with files on disk)
    if let Some(folder_path) = json_data.get("folder_path").and_then(|v| v.as_str()) {
        let path = std::path::PathBuf::from(folder_path);
        if path.exists() && path.is_dir() {
            tracing::debug!("envelope_unwrapper: found 'folder_path' key: {:?}", path);
            return UnwrappedContent::FolderPath(path);
        } else {
            tracing::warn!("envelope_unwrapper: folder_path does not exist or is not a directory: {:?}", path);
        }
    }

    // Check for binary DICOM wrapper (body_b64)
    if let Some(b64_str) = json_data.get("body_b64").and_then(|v| v.as_str()) {
        tracing::debug!("envelope_unwrapper: found 'body_b64' key, decoding binary DICOM");
        use base64::Engine;
        match base64::engine::general_purpose::STANDARD.decode(b64_str) {
            Ok(bytes) => return UnwrappedContent::Binary(bytes),
            Err(e) => {
                tracing::error!("envelope_unwrapper: failed to decode base64: {}", e);
                return UnwrappedContent::Empty;
            }
        }
    }

    // Extract the array of results from various envelope formats
    let results = extract_results_array(json_data);

    // Convert results to clean DICOM JSON objects
    let dicom_objects: Vec<Value> = results
        .into_iter()
        .filter_map(|item| extract_dicom_json(&item))
        .collect();

    if dicom_objects.is_empty() {
        UnwrappedContent::Empty
    } else {
        UnwrappedContent::DicomJson(dicom_objects)
    }
}

/// Extracts the results array from various envelope formats
fn extract_results_array(json_data: &Value) -> Vec<Value> {
    // Priority order for extracting results:
    // 1. "matches" key (C-FIND results)
    // 2. "instances" key (C-GET/C-MOVE results)
    // 3. "dicomweb_data" key (DICOMweb middleware format)
    // 4. Raw array
    // 5. Single object (wrap in array)

    if let Some(matches) = json_data.get("matches") {
        tracing::debug!("envelope_unwrapper: extracting from 'matches' key");
        return value_to_array(matches);
    }

    if let Some(instances) = json_data.get("instances") {
        tracing::debug!("envelope_unwrapper: extracting from 'instances' key");
        return value_to_array(instances);
    }

    if let Some(dicomweb_data) = json_data.get("dicomweb_data") {
        tracing::debug!("envelope_unwrapper: extracting from 'dicomweb_data' key");
        return value_to_array(dicomweb_data);
    }

    if json_data.is_array() {
        tracing::debug!("envelope_unwrapper: using raw array");
        return value_to_array(json_data);
    }

    if json_data.is_object() {
        // Check if this looks like an envelope (has non-DICOM keys) or raw DICOM JSON
        if let Some(obj) = json_data.as_object() {
            let has_envelope_keys = obj.keys().any(|k| {
                matches!(
                    k.as_str(),
                    "operation" | "success" | "dicomweb_response_type" | "dicomweb_metadata"
                )
            });

            if has_envelope_keys {
                tracing::debug!(
                    "envelope_unwrapper: object has envelope keys but no 'matches'/'dicomweb_data', returning empty"
                );
                return vec![];
            }
        }

        // Treat as single DICOM JSON object
        tracing::debug!("envelope_unwrapper: treating object as single DICOM JSON");
        return vec![json_data.clone()];
    }

    vec![]
}

/// Converts a Value to a Vec<Value>, handling both arrays and single objects
fn value_to_array(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(arr) => arr.clone(),
        Value::Null => vec![],
        other => vec![other.clone()],
    }
}

/// Extracts clean DICOM JSON from a single item (may be wrapped)
fn extract_dicom_json(item: &Value) -> Option<Value> {
    if item.is_null() {
        return None;
    }

    // Check for wrapper with "identifier" key
    if let Some(identifier) = item.get("identifier") {
        tracing::debug!("envelope_unwrapper: extracting from 'identifier' wrapper");
        return extract_dicom_json(identifier);
    }

    // If it's an object, filter to only DICOM tag keys
    if let Some(obj) = item.as_object() {
        let filtered: serde_json::Map<String, Value> = obj
            .iter()
            .filter(|(key, _)| is_dicom_tag_key(key))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        if filtered.is_empty() {
            tracing::debug!(
                "envelope_unwrapper: no DICOM tags found in object. Keys: {:?}",
                obj.keys().collect::<Vec<_>>()
            );
            return None;
        }

        tracing::debug!(
            "envelope_unwrapper: extracted {} DICOM tags",
            filtered.len()
        );
        return Some(Value::Object(filtered));
    }

    // Not a valid DICOM JSON object
    tracing::debug!("envelope_unwrapper: item is not an object, skipping");
    None
}

/// Check if a key looks like a DICOM tag (8 hex characters)
fn is_dicom_tag_key(key: &str) -> bool {
    key.len() == 8 && key.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_unwrap_raw_dicom_json() {
        let input = json!({
            "00100020": {"vr": "LO", "Value": ["12345"]},
            "00100010": {"vr": "PN", "Value": [{"Alphabetic": "DOE^JOHN"}]}
        });

        match unwrap_response(&input) {
            UnwrappedContent::DicomJson(items) => {
                assert_eq!(items.len(), 1);
                assert!(items[0].get("00100020").is_some());
                assert!(items[0].get("00100010").is_some());
            }
            _ => panic!("Expected DicomJson"),
        }
    }

    #[test]
    fn test_unwrap_dimse_envelope() {
        let input = json!({
            "operation": "find",
            "success": true,
            "matches": [
                {"00100020": {"vr": "LO", "Value": ["12345"]}},
                {"00100020": {"vr": "LO", "Value": ["67890"]}}
            ]
        });

        match unwrap_response(&input) {
            UnwrappedContent::DicomJson(items) => {
                assert_eq!(items.len(), 2);
            }
            _ => panic!("Expected DicomJson"),
        }
    }

    #[test]
    fn test_unwrap_dicomweb_envelope() {
        let input = json!({
            "dicomweb_response_type": "qido_json",
            "dicomweb_data": [
                {"00100020": {"vr": "LO", "Value": ["12345"]}}
            ],
            "dicomweb_metadata": {"has_results": true}
        });

        match unwrap_response(&input) {
            UnwrappedContent::DicomJson(items) => {
                assert_eq!(items.len(), 1);
                assert!(items[0].get("00100020").is_some());
                // Should not contain envelope keys
                assert!(items[0].get("dicomweb_response_type").is_none());
            }
            _ => panic!("Expected DicomJson"),
        }
    }

    #[test]
    fn test_unwrap_wrapper_struct() {
        let input = json!({
            "command": {"message_id": 1},
            "identifier": {
                "00100020": {"vr": "LO", "Value": ["12345"]}
            }
        });

        match unwrap_response(&input) {
            UnwrappedContent::DicomJson(items) => {
                assert_eq!(items.len(), 1);
                assert!(items[0].get("00100020").is_some());
                // Should not contain wrapper keys
                assert!(items[0].get("command").is_none());
            }
            _ => panic!("Expected DicomJson"),
        }
    }

    #[test]
    fn test_unwrap_mixed_keys_filters_non_dicom() {
        let input = json!({
            "00100020": {"vr": "LO", "Value": ["12345"]},
            "some_other_key": "value"
        });

        match unwrap_response(&input) {
            UnwrappedContent::DicomJson(items) => {
                assert_eq!(items.len(), 1);
                assert!(items[0].get("00100020").is_some());
                assert!(items[0].get("some_other_key").is_none());
            }
            _ => panic!("Expected DicomJson"),
        }
    }

    #[test]
    fn test_unwrap_empty_envelope() {
        let input = json!({
            "operation": "find",
            "success": true
            // No matches key
        });

        match unwrap_response(&input) {
            UnwrappedContent::Empty => {}
            _ => panic!("Expected Empty"),
        }
    }

    #[test]
    fn test_unwrap_null() {
        match unwrap_response(&Value::Null) {
            UnwrappedContent::Empty => {}
            _ => panic!("Expected Empty"),
        }
    }

    #[test]
    fn test_unwrap_instances_envelope() {
        // C-GET/C-MOVE response format uses "instances" instead of "matches"
        let input = json!({
            "operation": "get",
            "success": true,
            "instances": [
                {"00100020": {"vr": "LO", "Value": ["12345"]}}
            ],
            "folder_id": "test-folder",
            "file_count": 1
        });

        match unwrap_response(&input) {
            UnwrappedContent::DicomJson(items) => {
                assert_eq!(items.len(), 1);
                assert!(items[0].get("00100020").is_some());
            }
            _ => panic!("Expected DicomJson"),
        }
    }

    #[test]
    fn test_unwrap_folder_path() {
        // When folder_path exists, it should return FolderPath variant
        // Note: This test requires a real directory to exist
        let temp_dir = std::env::temp_dir();
        let input = json!({
            "operation": "get",
            "success": true,
            "folder_path": temp_dir.to_string_lossy(),
            "file_count": 1
        });

        match unwrap_response(&input) {
            UnwrappedContent::FolderPath(path) => {
                assert_eq!(path, temp_dir);
            }
            _ => panic!("Expected FolderPath"),
        }
    }

    #[test]
    fn test_is_dicom_tag_key() {
        assert!(is_dicom_tag_key("00100010"));
        assert!(is_dicom_tag_key("7FE00010"));
        assert!(is_dicom_tag_key("ABCDEF12"));

        assert!(!is_dicom_tag_key("0010001")); // Too short
        assert!(!is_dicom_tag_key("001000100")); // Too long
        assert!(!is_dicom_tag_key("0010001G")); // Invalid hex
        assert!(!is_dicom_tag_key("operation")); // Not hex
        assert!(!is_dicom_tag_key("dicomweb_data")); // Not hex
    }
}
