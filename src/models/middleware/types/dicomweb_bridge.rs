use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use base64::Engine;
use dicom_core::dictionary::{DataDictionary, VirtualVr};
use dicom_core::Tag;
use dicom_dictionary_std::StandardDataDictionary;
use dicom_pixeldata::image as img;
use dicom_pixeldata::PixelDecoder;
use img::ImageEncoder;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Bridge middleware that maps DICOMweb HTTP requests (QIDO/WADO) into DIMSE operations
/// and converts DICOM responses back to DICOMweb format.
///
/// LEFT side: Converts DICOMweb requests to DIMSE operations
/// RIGHT side: Converts DICOM backend responses to DICOMweb JSON/binary
#[derive(Default, Debug)]
pub struct DicomwebBridgeMiddleware;

impl DicomwebBridgeMiddleware {
    pub fn new() -> Self {
        Self
    }

    // --- LEFT SIDE HELPERS (DICOMweb → DICOM) ---

    fn set_backend_path(metadata: &mut HashMap<String, String>, op: &str) {
        // Store intended DIMSE operation without overwriting the routing path
        metadata.insert("dimse_op".to_string(), op.to_string());
        // Ensure backends are not skipped
        metadata.insert("skip_backends".to_string(), "false".to_string());
    }

    fn clear_endpoint_response(nd: &mut Value) {
        if let Some(obj) = nd.as_object_mut() {
            obj.remove("response");
        }
    }


    fn make_ident_entry(vr: &str, vals: Vec<String>) -> Value {
        json!({ "vr": vr, "Value": vals })
    }

    fn add_tag(map: &mut serde_json::Map<String, Value>, tag: &str, vr: &str, vals: Vec<String>) {
        map.insert(tag.to_string(), Self::make_ident_entry(vr, vals));
    }

    // --- RIGHT SIDE HELPERS (DICOM → DICOMweb) ---

    fn set_dicomweb_data(
        envelope: &mut RequestEnvelope<Value>,
        response_type: &str,
        data: Value,
        metadata: Option<serde_json::Map<String, Value>>,
    ) {
        // Set the processed data in normalized_data for the endpoint to handle HTTP concerns
        let nd = envelope
            .normalized_data
            .take()
            .unwrap_or_else(|| serde_json::json!({}));
        let mut map = nd.as_object().cloned().unwrap_or_default();
        
        // Set the response type for endpoint to determine proper HTTP handling
        map.insert("dicomweb_response_type".to_string(), Value::String(response_type.to_string()));
        map.insert("dicomweb_data".to_string(), data);
        
        // Include any additional metadata
        if let Some(meta) = metadata {
            map.insert("dicomweb_metadata".to_string(), Value::Object(meta));
        }
        
        let new_nd = Value::Object(map);
        envelope.normalized_data = Some(new_nd.clone());
        envelope.original_data = new_nd;
    }

    fn build_qido_json_from_matches(matches: &Value, includefield: Option<&Vec<String>>) -> Value {
        // Convert identifier JSON objects to full DICOMweb JSON objects
        // Filter attributes based on includefield parameter if provided
        match matches {
            Value::Array(arr) => {
                let mut dicomweb_objects = Vec::new();
                for item in arr {
                    let processed_item = Self::process_item(includefield, item);
                    dicomweb_objects.push(processed_item);
                }
                Value::Array(dicomweb_objects)
            }
            other => {
                // Single object case
                let processed_item = Self::process_item(includefield, other);
                Value::Array(vec![processed_item])
            }
        }
    }

    fn process_item(includefield: Option<&Vec<String>>, item: &Value) -> Value {
        let processed_item = match includefield {
            Some(_fields) => {
                // If filtering is requested, work with the identifier as-is instead of
                // converting to full JSON first, to avoid including unwanted attributes
                Self::filter_dicom_json(item, includefield)
            }
            None => {
                // No filtering requested, convert to full DICOMweb JSON
                if let Ok(dicom_obj) = dicom_json_tool::json_value_to_identifier(item) {
                    if let Ok(full_json) =
                        dicom_json_tool::identifier_to_json_value(&dicom_obj)
                    {
                        full_json
                    } else {
                        item.clone()
                    }
                } else {
                    item.clone()
                }
            }
        };
        processed_item
    }

    /// Filter DICOM JSON attributes based on includefield parameter
    /// If includefield is None, return all attributes (except pixel data)
    /// If includefield is Some, return only the specified attributes
    fn filter_dicom_json(json: &Value, includefield: Option<&Vec<String>>) -> Value {
        match json {
            Value::Object(obj) => {
                let mut filtered = serde_json::Map::new();

                match includefield {
                    Some(fields) => {
                        // Only include specified fields
                        for field in fields {
                            if let Some(value) = obj.get(field) {
                                filtered.insert(field.clone(), value.clone());
                            }
                        }
                    }
                    None => {
                        // Include all attributes except pixel data and large binary data
                        for (key, value) in obj {
                            if !Self::should_exclude_dicom_tag(key) {
                                filtered.insert(key.clone(), value.clone());
                            }
                        }
                    }
                }

                Value::Object(filtered)
            }
            _ => json.clone(),
        }
    }

    /// Helper function to determine if a DICOM tag should be excluded from JSON responses
    /// This excludes pixel data and other large binary attributes by default
    fn should_exclude_dicom_tag(tag_str: &str) -> bool {
        match tag_str {
            // Pixel Data
            "7FE00010" => true,
            // Overlay Data (group 6000-60FF)
            tag if tag.len() == 8 => {
                if let Ok(group) = u16::from_str_radix(&tag[0..4], 16) {
                    (0x6000..=0x60FF).contains(&group)
                } else {
                    false
                }
            }
            // Private Creator tags and other large binary data can be added here
            _ => false,
        }
    }

    /// Convert DICOM tag name or hex string to hex format using dicom-rs StandardDataDictionary
    /// Examples: "PatientName" -> "00100010", "StudyDate" -> "00080020"
    fn dicom_name_to_hex(name_or_hex: &str) -> String {
        // If it's already in hex format (8 characters), return as-is
        if name_or_hex.len() == 8 && name_or_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return name_or_hex.to_uppercase();
        }

        // Use dicom-rs StandardDataDictionary to look up tag by name
        if let Some(entry) = StandardDataDictionary.by_name(name_or_hex) {
            let tag = entry.tag.inner(); // Get the inner Tag from TagRange
            return format!("{:08X}", (tag.0 as u32) << 16 | tag.1 as u32);
        }

        // If not found, return the original (might already be a hex tag)
        name_or_hex.to_uppercase()
    }

    /// Infer VR (Value Representation) for a DICOM tag using dicom-rs StandardDataDictionary
    fn infer_vr_for_tag(tag_hex: &str) -> String {
        // Parse hex tag back to Tag struct
        if let (Ok(group), Ok(element)) = (
            u16::from_str_radix(&tag_hex[0..4], 16),
            u16::from_str_radix(&tag_hex[4..8], 16),
        ) {
            let tag = Tag(group, element);
            // Use dicom-rs StandardDataDictionary to get VR
            if let Some(entry) = StandardDataDictionary.by_tag(tag) {
                // Convert VirtualVr to string and format consistently
                let vr_str = match entry.vr {
                    VirtualVr::Exact(vr) => format!("{:?}", vr),
                    _ => "LO".to_string(),
                };
                return format!("Exact({})", vr_str);
            }
        }

        // Fallback to Exact(LO) for unknown tags
        "Exact(LO)".to_string()
    }

    /// Add a return key to the identifier if not already present
    fn add_return_key_if_missing(ident: &mut serde_json::Map<String, Value>, field_name: &str) {
        let tag_hex = Self::dicom_name_to_hex(field_name);
        // Only add if not already present (preserves search criteria)
        if !ident.contains_key(&tag_hex) {
            let vr = Self::infer_vr_for_tag(&tag_hex);
            Self::add_tag(ident, &tag_hex, &vr, vec![]);
        }
    }

    /// Add default return keys when no includefield is specified
    fn add_default_return_keys(ident: &mut serde_json::Map<String, Value>, level: &str) {
        let add_tag_by_name = |ident: &mut serde_json::Map<String, Value>, name: &str| {
            let tag_hex = Self::dicom_name_to_hex(name);
            let vr = Self::infer_vr_for_tag(&tag_hex);
            Self::add_tag(ident, &tag_hex, &vr, vec![]);
        };

        match level {
            "study" => {
                // Default study-level return keys for full metadata
                add_tag_by_name(ident, "StudyInstanceUID");
                add_tag_by_name(ident, "StudyDate");
                add_tag_by_name(ident, "ModalitiesInStudy");
                add_tag_by_name(ident, "PatientID");
                add_tag_by_name(ident, "PatientName");
                add_tag_by_name(ident, "StudyDescription");
                add_tag_by_name(ident, "AccessionNumber");
            }
            "series" => {
                // Default series-level return keys
                add_tag_by_name(ident, "SeriesInstanceUID");
                add_tag_by_name(ident, "Modality");
                add_tag_by_name(ident, "SeriesDescription");
                add_tag_by_name(ident, "SeriesNumber");
            }
            "instance" => {
                // Default instance-level return keys
                add_tag_by_name(ident, "SOPInstanceUID");
                add_tag_by_name(ident, "InstanceNumber");
                add_tag_by_name(ident, "SOPClassUID");
            }
            _ => {}
        }
    }

    fn read_instance_bytes(folder_path: &str) -> Result<Vec<Vec<u8>>, String> {
        let mut parts: Vec<Vec<u8>> = Vec::new();
        let base = PathBuf::from(folder_path);
        if !base.exists() {
            return Err(format!("folder not found: {}", folder_path));
        }
        for entry in fs::read_dir(&base).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.is_file() {
                // Include .dcm and other files
                let bytes = fs::read(&p).map_err(|e| e.to_string())?;
                parts.push(bytes);
            }
        }
        if parts.is_empty() {
            return Err("no files found".to_string());
        }
        Ok(parts)
    }

    fn build_multipart(parts: Vec<Vec<u8>>) -> (String, Vec<u8>) {
        let boundary = format!("dicomweb_{}", uuid::Uuid::new_v4());
        let mut buf: Vec<u8> = Vec::new();
        for part in parts {
            buf.extend_from_slice(format!("--{}\r\n", &boundary).as_bytes());
            buf.extend_from_slice(b"Content-Type: application/dicom\r\n\r\n");
            buf.extend_from_slice(&part);
            buf.extend_from_slice(b"\r\n");
        }
        buf.extend_from_slice(format!("--{}--\r\n", &boundary).as_bytes());
        (boundary, buf)
    }
}

#[async_trait::async_trait]
impl Middleware for DicomwebBridgeMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        let method = envelope.request_details.method.to_uppercase();
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();
        let qp = &envelope.request_details.query_params;

        // Only act on GET requests from DICOMweb endpoints
        if method != "GET" {
            return Ok(envelope);
        }

        // Parse path segments (already relative to path_prefix)
        let parts: Vec<&str> = subpath.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Ok(envelope);
        }

        // Build DICOM identifier JSON using hex tags
        let mut ident = serde_json::Map::<String, Value>::new();

        // Process all query parameters (except special ones like includefield)
        for (param_name, param_values) in qp {
            // Skip special DICOMweb parameters that aren't DICOM tags
            if matches!(param_name.as_str(), "includefield" | "limit" | "offset" | "fuzzymatching") {
                continue;
            }
            
            // Convert parameter name to DICOM hex tag
            let tag_hex = Self::dicom_name_to_hex(param_name);
            let vr = Self::infer_vr_for_tag(&tag_hex);
            
            // Use all values for this parameter (DICOMweb allows multiple values)
            let values: Vec<String> = param_values.iter().cloned().collect();
            if !values.is_empty() {
                Self::add_tag(&mut ident, &tag_hex, &vr, values);
            }
        }

        // Parse includefield query parameter for attribute filtering
        let includefield: Option<Vec<String>> = qp
            .get("includefield")
            .map(|values| values.iter().map(|s| s.to_string()).collect());

        // Store includefield parameter in request metadata for right-side processing
        if let Some(ref fields) = includefield {
            let fields_json = serde_json::to_string(fields).unwrap_or_default();
            envelope
                .request_details
                .metadata
                .insert("dicomweb_includefield".to_string(), fields_json);
        }

        // Helper function to add return keys based on includefield or defaults
        let add_return_keys = |ident: &mut serde_json::Map<String, Value>, level: &str| {
            match includefield {
                Some(ref fields) => {
                    // Add return keys for each field in includefield
                    for field in fields {
                        Self::add_return_key_if_missing(ident, field);
                    }
                }
                None => {
                    // No includefield specified - add default return keys for the level
                    Self::add_default_return_keys(ident, level);
                }
            }
        };

        // Route-based mapping
        let mut op = None::<&str>;
        match parts.as_slice() {
            // QIDO: /studies
            ["studies"] => {
                op = Some("find");
                add_return_keys(&mut ident, "study");
            }
            // QIDO: /studies/{study}
            ["studies", study_uid] => {
                op = Some("find");
                let vr = Self::infer_vr_for_tag("0020000D");
                Self::add_tag(&mut ident, "0020000D", &vr, vec![(*study_uid).to_string()]);
                add_return_keys(&mut ident, "study");
            }
            // QIDO: /studies/{study}/series
            ["studies", study_uid, "series"] => {
                op = Some("find");
                let vr = Self::infer_vr_for_tag("0020000D");
                Self::add_tag(&mut ident, "0020000D", &vr, vec![(*study_uid).to_string()]);
                add_return_keys(&mut ident, "series");
            }
            // QIDO: /studies/{study}/series/{series} (specific series)
            ["studies", study_uid, "series", series_uid] => {
                op = Some("find");
                let study_vr = Self::infer_vr_for_tag("0020000D");
                let series_vr = Self::infer_vr_for_tag("0020000E");
                Self::add_tag(&mut ident, "0020000D", &study_vr, vec![(*study_uid).to_string()]);
                Self::add_tag(&mut ident, "0020000E", &series_vr, vec![(*series_uid).to_string()]);
                add_return_keys(&mut ident, "series");
            }
            // QIDO: /studies/{study}/series/{series}/instances
            ["studies", study_uid, "series", series_uid, "instances"] => {
                op = Some("find");
                let study_vr = Self::infer_vr_for_tag("0020000D");
                let series_vr = Self::infer_vr_for_tag("0020000E");
                Self::add_tag(&mut ident, "0020000D", &study_vr, vec![(*study_uid).to_string()]);
                Self::add_tag(&mut ident, "0020000E", &series_vr, vec![(*series_uid).to_string()]);
                add_return_keys(&mut ident, "instance");
            }
            // Handle /studies/{study}/series/{series}/instances/{instance} for both QIDO and WADO
            ["studies", study_uid, "series", series_uid, "instances", instance_uid] => {
                // Check if this is a QIDO query (exactly 6 parts) vs WADO (for instance retrieval)
                let subpath = envelope
                    .request_details
                    .metadata
                    .get("path")
                    .cloned()
                    .unwrap_or_default();
                let parts_count = subpath.split('/').filter(|s| !s.is_empty()).count();
                
                if parts_count == 6 { // exactly /studies/{study}/series/{series}/instances/{instance}
                    op = Some("find"); // QIDO find for specific instance
                    let study_vr = Self::infer_vr_for_tag("0020000D");
                    let series_vr = Self::infer_vr_for_tag("0020000E");
                    let instance_vr = Self::infer_vr_for_tag("00080018");
                    Self::add_tag(&mut ident, "0020000D", &study_vr, vec![(*study_uid).to_string()]);
                    Self::add_tag(&mut ident, "0020000E", &series_vr, vec![(*series_uid).to_string()]);
                    Self::add_tag(&mut ident, "00080018", &instance_vr, vec![(*instance_uid).to_string()]);
                    add_return_keys(&mut ident, "instance");
                } else {
                    // For WADO instance retrieval (when path has more than 6 parts or no suffix)
                    // This handles cases where instance route is used for binary retrieval
                    op = Some("get"); 
                    let study_vr = Self::infer_vr_for_tag("0020000D");
                    let series_vr = Self::infer_vr_for_tag("0020000E");
                    let instance_vr = Self::infer_vr_for_tag("00080018");
                    Self::add_tag(&mut ident, "0020000D", &study_vr, vec![(*study_uid).to_string()]);
                    Self::add_tag(&mut ident, "0020000E", &series_vr, vec![(*series_uid).to_string()]);
                    Self::add_tag(&mut ident, "00080018", &instance_vr, vec![(*instance_uid).to_string()]);
                }
            }
            // WADO metadata: /studies/{study}/metadata
            ["studies", study_uid, "metadata"] => {
                op = Some("find");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                add_return_keys(&mut ident, "study");
            }
            // WADO metadata: /studies/{study}/series/{series}/metadata
            ["studies", study_uid, "series", series_uid, "metadata"] => {
                op = Some("find");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                Self::add_tag(
                    &mut ident,
                    "0020000E",
                    "UI",
                    vec![(*series_uid).to_string()],
                );
                add_return_keys(&mut ident, "series");
            }
            // WADO metadata: /studies/{study}/series/{series}/instances/{instance}/metadata
            ["studies", study_uid, "series", series_uid, "instances", instance_uid, "metadata"] => {
                op = Some("find");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                Self::add_tag(
                    &mut ident,
                    "0020000E",
                    "UI",
                    vec![(*series_uid).to_string()],
                );
                Self::add_tag(
                    &mut ident,
                    "00080018",
                    "UI",
                    vec![(*instance_uid).to_string()],
                );
                add_return_keys(&mut ident, "instance");
            }
            // WADO: frames (map to get at instance level)
            ["studies", study_uid, "series", series_uid, "instances", instance_uid, "frames", _frames] =>
            {
                op = Some("get");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                Self::add_tag(
                    &mut ident,
                    "0020000E",
                    "UI",
                    vec![(*series_uid).to_string()],
                );
                Self::add_tag(
                    &mut ident,
                    "00080018",
                    "UI",
                    vec![(*instance_uid).to_string()],
                );
            }
            // WADO bulkdata (map to get; actual subresource ignored for now)
            ["bulkdata", ..] => {
                op = Some("get");
            }
            _ => {}
        }

        if let Some(op_name) = op {
            
            // Ensure metadata prepared for backend
            Self::set_backend_path(&mut envelope.request_details.metadata, op_name);

            // Clear any response set by upstream endpoint
            let mut nd = envelope
                .normalized_data
                .clone()
                .unwrap_or_else(|| json!({}));
            Self::clear_endpoint_response(&mut nd);

            // Attach identifier for backend to consume
            if let Some(obj) = nd.as_object_mut() {
                obj.insert("dimse_identifier".to_string(), Value::Object(ident));
            } else {
                let mut map = serde_json::Map::new();
                map.insert("dimse_identifier".to_string(), Value::Object(ident));
                nd = Value::Object(map);
            }
            envelope.normalized_data = Some(nd);
        }

        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Detect DICOMweb routes from metadata.path
        // Prefer the full HTTP path (with optional query) because another middleware may override
        // metadata.path with an operation name (e.g., "get"/"find").
        let mut raw_path = envelope
            .request_details
            .metadata
            .get("full_path")
            .cloned()
            .unwrap_or_else(|| {
                envelope
                    .request_details
                    .metadata
                    .get("path")
                    .cloned()
                    .unwrap_or_default()
            });
        // Strip query string if present
        if let Some((p, _)) = raw_path.split_once('?') {
            raw_path = p.to_string();
        }
        // Extract the DICOMweb subpath segment beginning at "studies/" if available
        let path = if let Some(idx) = raw_path.find("studies/") {
            raw_path[idx..].to_string()
        } else if let Some(idx) = raw_path.find("bulkdata/") {
            raw_path[idx..].to_string()
        } else {
            raw_path
        };
        let nd = envelope
            .normalized_data
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));

        let operation = nd.get("operation").and_then(|v| v.as_str()).unwrap_or("");
        let success = nd.get("success").and_then(|v| v.as_bool()).unwrap_or(false);

        // Parse includefield from request metadata (set by left-side middleware)
        let includefield: Option<Vec<String>> = envelope
            .request_details
            .metadata
            .get("dicomweb_includefield")
            .and_then(|json_str| serde_json::from_str(json_str).ok());

        // QIDO lists -> DICOMweb JSON data
        if operation == "find" {
            let matches_val = nd.get("matches").cloned().unwrap_or(Value::Array(vec![]));
            let json = Self::build_qido_json_from_matches(&matches_val, includefield.as_ref());
            
            
            Self::set_dicomweb_data(&mut envelope, "qido_json", json, None);
            return Ok(envelope);
        }

        // WADO metadata -> DICOMweb JSON data
        if path.ends_with("/metadata") {
            // Prefer instances array; otherwise matches
            let datasets = nd.get("instances").or_else(|| nd.get("matches"));
            let raw_json = datasets.cloned().unwrap_or(Value::Array(vec![]));

            // Apply includefield filtering to metadata responses
            let filtered_json = match raw_json {
                Value::Array(arr) => {
                    let filtered_arr: Vec<Value> = arr
                        .iter()
                        .map(|item| Self::filter_dicom_json(item, includefield.as_ref()))
                        .collect();
                    Value::Array(filtered_arr)
                }
                other => Self::filter_dicom_json(&other, includefield.as_ref()),
            };

            Self::set_dicomweb_data(&mut envelope, "wado_metadata", filtered_json, None);
            return Ok(envelope);
        }

        // WADO instance retrieval -> multipart DICOM data
        if operation == "get" && path.contains("/instances/") && !path.contains("/frames/") {
            if let Some(folder_path) = nd.get("folder_path").and_then(|v| v.as_str()) {
                match Self::read_instance_bytes(folder_path) {
                    Ok(parts) => {
                        let (boundary, body_bytes) = Self::build_multipart(parts);
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&body_bytes);
                        
                        let mut metadata = serde_json::Map::new();
                        metadata.insert("boundary".to_string(), Value::String(boundary));
                        metadata.insert("body_b64".to_string(), Value::String(b64));
                        
                        Self::set_dicomweb_data(&mut envelope, "wado_instance", Value::Null, Some(metadata));
                        return Ok(envelope);
                    }
                    Err(_e) => {
                        // fall through and do nothing; let endpoint serialize default JSON
                    }
                }
            }
        }

        // WADO frames -> decode and encode image data
        if operation == "get" && (path.contains("/frames/") || path.contains("frames/")) {
            // Accept negotiation - store preferred format for endpoint to use
            let accept = envelope
                .request_details
                .headers
                .get("accept")
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            let want_jpeg = accept.contains("image/jpeg") || accept.contains("*/*");
            let want_png = accept.contains("image/png");
            let content_type = if want_jpeg {
                "image/jpeg"
            } else if want_png {
                "image/png"
            } else {
                "image/jpeg"
            };

            if let Some(folder_path) = nd.get("folder_path").and_then(|v| v.as_str()) {
                // Parse instance UID and frame numbers from path
                let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                // expected: studies/{study}/series/{series}/instances/{instance}/frames/{frames}
                let instance_uid = parts.get(5).copied().unwrap_or("");
                let frames_str = parts.get(7).copied().unwrap_or("");
                let frame_numbers: Vec<usize> = frames_str
                    .split(',')
                    .filter_map(|s| s.parse::<usize>().ok())
                    .collect();

                // Pick instance file path from folder_path
                let instance_path = {
                    // try to match SOPInstanceUID; fallback to first file
                    let base = std::path::PathBuf::from(folder_path);
                    let mut chosen: Option<std::path::PathBuf> = None;
                    if let Ok(rd) = std::fs::read_dir(&base) {
                        for e in rd.flatten() {
                            let p = e.path();
                            if !p.is_file() {
                                continue;
                            }
                            if let Ok(obj) = dicom_object::open_file(&p) {
                                if let Ok(el) = obj.element_by_name("SOPInstanceUID") {
                                    if let Ok(uid) = el.to_str() {
                                        if uid == instance_uid {
                                            chosen = Some(p.clone());
                                            break;
                                        }
                                    }
                                }
                            }
                            if chosen.is_none() {
                                chosen = Some(p.clone());
                            }
                        }
                    }
                    chosen
                };

                if let Some(ipath) = instance_path {
                    // Decode frames using dicom-pixeldata 0.9 API
                    let obj = dicom_object::open_file(&ipath)
                        .map_err(|e| Error::from(format!("open dicom: {}", e)))?;
                    match obj.decode_pixel_data() {
                        Ok(pixel_data) => {
                            let mut images: Vec<Vec<u8>> = Vec::new();
                            for f in frame_numbers.iter() {
                                let idx = f.saturating_sub(1) as u32;
                                match pixel_data.to_dynamic_image(idx) {
                                    Ok(dyn_img) => {
                                        let mut buf: Vec<u8> = Vec::new();
                                        if content_type == "image/jpeg" {
                                            let mut enc =
                                                img::codecs::jpeg::JpegEncoder::new_with_quality(
                                                    &mut buf, 90,
                                                );
                                            if let Err(e) = enc.encode_image(&dyn_img) {
                                                return Err(Error::from(format!(
                                                    "jpeg encode: {}",
                                                    e
                                                )));
                                            }
                                        } else {
                                            let enc = img::codecs::png::PngEncoder::new(&mut buf);
                                            if let Err(e) = enc.write_image(
                                                dyn_img.as_bytes(),
                                                dyn_img.width(),
                                                dyn_img.height(),
                                                dyn_img.color().into(),
                                            ) {
                                                return Err(Error::from(format!(
                                                    "png encode: {}",
                                                    e
                                                )));
                                            }
                                        }
                                        images.push(buf);
                                    }
                                    Err(e) => return Err(Error::from(format!("to image: {}", e))),
                                }
                            }

                            if images.len() == 1 {
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&images[0]);
                                let mut metadata = serde_json::Map::new();
                                metadata.insert("content_type".to_string(), Value::String(content_type.to_string()));
                                metadata.insert("body_b64".to_string(), Value::String(b64));
                                metadata.insert("is_single_frame".to_string(), Value::Bool(true));
                                
                                Self::set_dicomweb_data(&mut envelope, "wado_frames", Value::Null, Some(metadata));
                                return Ok(envelope);
                            } else if !images.is_empty() {
                                let boundary = format!("dicomweb_{}", uuid::Uuid::new_v4());
                                let mut body: Vec<u8> = Vec::new();
                                for img in images {
                                    body.extend_from_slice(
                                        format!("--{}\r\n", &boundary).as_bytes(),
                                    );
                                    body.extend_from_slice(
                                        format!("Content-Type: {}\r\n\r\n", content_type)
                                            .as_bytes(),
                                    );
                                    body.extend_from_slice(&img);
                                    body.extend_from_slice(b"\r\n");
                                }
                                body.extend_from_slice(format!("--{}--\r\n", &boundary).as_bytes());
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&body);
                                
                                let mut metadata = serde_json::Map::new();
                                metadata.insert("content_type".to_string(), Value::String(content_type.to_string()));
                                metadata.insert("boundary".to_string(), Value::String(boundary));
                                metadata.insert("body_b64".to_string(), Value::String(b64));
                                metadata.insert("is_single_frame".to_string(), Value::Bool(false));
                                
                                Self::set_dicomweb_data(&mut envelope, "wado_frames", Value::Null, Some(metadata));
                                return Ok(envelope);
                            }
                        }
                        Err(_e) => {
                            // Unsupported TS or decoding error - let endpoint handle the error response
                            let mut metadata = serde_json::Map::new();
                            metadata.insert("error".to_string(), Value::String("UnsupportedTransferSyntax".to_string()));
                            metadata.insert("message".to_string(), Value::String("Unable to decode frames for requested instance".to_string()));
                            
                            Self::set_dicomweb_data(&mut envelope, "wado_frames_error", Value::Null, Some(metadata));
                            return Ok(envelope);
                        }
                    }
                }
            }
        }

        // If backend failed for non-frame requests, do not override
        if !success {
            return Ok(envelope);
        }

        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{RequestEnvelope, RequestDetails};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_left_includefield_adds_return_keys() {
        let bridge = DicomwebBridgeMiddleware::new();

        // Create a mock request envelope with includefield parameters
        let mut query_params: HashMap<String, Vec<String>> = HashMap::new();
        query_params.insert(
            "includefield".to_string(),
            vec![
                "PatientName".to_string(),
                "StudyDate".to_string(),
                "00080020".to_string(), // StudyDate as hex tag - should not duplicate
            ],
        );

        let mut metadata: HashMap<String, String> = HashMap::new();
        metadata.insert("path".to_string(), "studies".to_string());

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/api/dicom/studies".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params,
            cache_status: None,
            metadata,
        };

        let envelope = RequestEnvelope {
            request_details,
            original_data: serde_json::json!({}),
            normalized_data: None,
            normalized_snapshot: None,
        };

        // Process through the middleware
        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed_envelope = result.unwrap();
        let nd = processed_envelope.normalized_data.unwrap();
        let identifier = nd.get("dimse_identifier").unwrap().as_object().unwrap();

        // Verify that includefield tags are present as return keys (empty values)
        assert!(
            identifier.contains_key("00100010"),
            "PatientName should be present"
        );
        assert!(
            identifier.contains_key("00080020"),
            "StudyDate should be present"
        );

        // Check that return keys have empty Value arrays
        let patient_name = identifier.get("00100010").unwrap();
        let value_array = patient_name.get("Value").unwrap().as_array().unwrap();
        assert!(
            value_array.is_empty(),
            "Return keys should have empty Value arrays"
        );

        // Verify includefield is stored in metadata for right-side processing
        assert!(processed_envelope
            .request_details
            .metadata
            .contains_key("dicomweb_includefield"));
        
        // Verify that dimse_op is set for backend processing
        assert_eq!(
            processed_envelope.request_details.metadata.get("dimse_op"),
            Some(&"find".to_string())
        );
        
        // Verify skip_backends is set to false
        assert_eq!(
            processed_envelope.request_details.metadata.get("skip_backends"),
            Some(&"false".to_string())
        );
    }

    #[tokio::test]
    async fn test_left_no_includefield_uses_defaults() {
        let bridge = DicomwebBridgeMiddleware::new();

        // Create a mock request envelope without includefield
        let mut metadata: HashMap<String, String> = HashMap::new();
        metadata.insert("path".to_string(), "studies".to_string());

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/api/dicom/studies".to_string(), 
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata,
        };

        let envelope = RequestEnvelope {
            request_details,
            original_data: serde_json::json!({}),
            normalized_data: None,
            normalized_snapshot: None,
        };

        // Process through the middleware
        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed_envelope = result.unwrap();
        let nd = processed_envelope.normalized_data.unwrap();
        let identifier = nd.get("dimse_identifier").unwrap().as_object().unwrap();

        // Verify default study-level return keys are present
        assert!(
            identifier.contains_key("0020000D"),
            "StudyInstanceUID should be present"
        );
        assert!(
            identifier.contains_key("00080020"),
            "StudyDate should be present"
        );
        assert!(
            identifier.contains_key("00100020"),
            "PatientID should be present"
        );
        assert!(
            identifier.contains_key("00100010"),
            "PatientName should be present"
        );

        // Verify no includefield metadata is stored
        assert!(!processed_envelope
            .request_details
            .metadata
            .contains_key("dicomweb_includefield"));
        
        // Verify that dimse_op is set for backend processing
        assert_eq!(
            processed_envelope.request_details.metadata.get("dimse_op"),
            Some(&"find".to_string())
        );
    }

    #[tokio::test]
    async fn test_left_includefield_preserves_search_criteria() {
        let bridge = DicomwebBridgeMiddleware::new();

        // Create a mock request with both search params and includefield
        let mut query_params: HashMap<String, Vec<String>> = HashMap::new();
        query_params.insert("PatientID".to_string(), vec!["12345".to_string()]);
        query_params.insert(
            "includefield".to_string(),
            vec!["PatientID".to_string(), "PatientName".to_string()],
        );

        let mut metadata: HashMap<String, String> = HashMap::new();
        metadata.insert("path".to_string(), "studies".to_string());

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/api/dicom/studies".to_string(),
            headers: HashMap::new(), 
            cookies: HashMap::new(),
            query_params,
            cache_status: None,
            metadata,
        };

        let envelope = RequestEnvelope {
            request_details,
            original_data: serde_json::json!({}),
            normalized_data: None,
            normalized_snapshot: None,
        };

        // Process through the middleware
        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed_envelope = result.unwrap();
        let nd = processed_envelope.normalized_data.unwrap();
        let identifier = nd.get("dimse_identifier").unwrap().as_object().unwrap();

        // Verify PatientID has search value, not empty return key
        let patient_id = identifier.get("00100020").unwrap();
        let value_array = patient_id.get("Value").unwrap().as_array().unwrap();
        assert_eq!(value_array.len(), 1);
        assert_eq!(value_array[0].as_str().unwrap(), "12345");

        // Verify PatientName is added as return key
        let patient_name = identifier.get("00100010").unwrap();
        let name_value_array = patient_name.get("Value").unwrap().as_array().unwrap();
        assert!(
            name_value_array.is_empty(),
            "PatientName should be return key with empty value"
        );
    }
    
    #[tokio::test]
    async fn test_right_qido_transformation() {
        let bridge = DicomwebBridgeMiddleware::new();
        
        // Create a mock right-side request with DICOM find results
        let mut metadata: HashMap<String, String> = HashMap::new();
        metadata.insert("path".to_string(), "studies".to_string());
        metadata.insert("full_path".to_string(), "/dicomweb/studies".to_string());
        
        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/dicomweb/studies".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata,
        };
        
        // Mock DICOM find results
        let matches = serde_json::json!([
            {
                "00100020": {"vr": "LO", "Value": ["12345"]},
                "00100010": {"vr": "PN", "Value": ["Doe^John"]},
                "0020000D": {"vr": "UI", "Value": ["1.2.3.4.5"]}
            }
        ]);
        
        let normalized_data = serde_json::json!({
            "operation": "find",
            "success": true,
            "matches": matches
        });
        
        let envelope = RequestEnvelope {
            request_details,
            original_data: serde_json::json!({}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };
        
        // Process through the right-side middleware
        let result = bridge.right(envelope).await;
        assert!(result.is_ok());
        
        let processed_envelope = result.unwrap();
        let nd = processed_envelope.normalized_data.unwrap();
        
        // Verify that DICOMweb response type is set
        assert_eq!(
            nd.get("dicomweb_response_type").and_then(|v| v.as_str()),
            Some("qido_json")
        );
        
        // Verify DICOMweb data is present
        assert!(nd.get("dicomweb_data").is_some());
        
        // Verify metadata indicates results are present
        let metadata = nd.get("dicomweb_metadata").and_then(|v| v.as_object());
        assert!(metadata.is_some());
        assert_eq!(
            metadata.unwrap().get("has_results").and_then(|v| v.as_bool()),
            Some(true)
        );
    }
    
    #[tokio::test]
    async fn test_left_processes_all_query_parameters() {
        let bridge = DicomwebBridgeMiddleware::new();
        
        // Create a mock request with various DICOM query parameters
        let mut query_params: HashMap<String, Vec<String>> = HashMap::new();
        query_params.insert("PatientName".to_string(), vec!["Doe^John".to_string()]);
        query_params.insert("StudyDate".to_string(), vec!["20231015".to_string()]);
        query_params.insert("Modality".to_string(), vec!["CT".to_string(), "MR".to_string()]); // Multiple values
        query_params.insert("StudyDescription".to_string(), vec!["Brain Study".to_string()]);
        query_params.insert("SeriesNumber".to_string(), vec!["1".to_string()]);
        query_params.insert("includefield".to_string(), vec!["PatientName".to_string()]); // Should be skipped
        query_params.insert("limit".to_string(), vec!["100".to_string()]); // Should be skipped
        
        let mut metadata: HashMap<String, String> = HashMap::new();
        metadata.insert("path".to_string(), "studies".to_string());
        
        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/dicomweb/studies".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params,
            cache_status: None,
            metadata,
        };
        
        let envelope = RequestEnvelope {
            request_details,
            original_data: serde_json::json!({}),
            normalized_data: None,
            normalized_snapshot: None,
        };
        
        // Process through the middleware
        let result = bridge.left(envelope).await;
        assert!(result.is_ok());
        
        let processed_envelope = result.unwrap();
        let nd = processed_envelope.normalized_data.unwrap();
        let identifier = nd.get("dimse_identifier").unwrap().as_object().unwrap();
        
        // Verify all query parameters were processed
        assert!(identifier.contains_key("00100010"), "PatientName should be present");
        assert!(identifier.contains_key("00080020"), "StudyDate should be present");
        assert!(identifier.contains_key("00080060"), "Modality should be present");
        assert!(identifier.contains_key("00081030"), "StudyDescription should be present");
        assert!(identifier.contains_key("00200011"), "SeriesNumber should be present");
        
        // Verify multiple values are preserved
        let modality_values = identifier.get("00080060")
            .and_then(|m| m.get("Value"))
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(modality_values.len(), 2);
        assert!(modality_values.contains(&serde_json::json!("CT")));
        assert!(modality_values.contains(&serde_json::json!("MR")));
        
        // Verify special parameters were skipped (not present as DICOM tags)
        // These would have nonsensical hex representations if processed
        let all_keys: Vec<&String> = identifier.keys().collect();
        assert!(!all_keys.iter().any(|k| k.starts_with("include")), "includefield should not be processed as DICOM tag");
        assert!(!all_keys.iter().any(|k| k.starts_with("limit")), "limit should not be processed as DICOM tag");
    }
}
