use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use base64::Engine;
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

    fn qp_first(qp: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
        qp.get(key).and_then(|v| v.first()).map(|s| s.to_string())
    }

    fn make_ident_entry(vr: &str, vals: Vec<String>) -> Value {
        json!({ "vr": vr, "Value": vals })
    }

    fn add_tag(map: &mut serde_json::Map<String, Value>, tag: &str, vr: &str, vals: Vec<String>) {
        map.insert(tag.to_string(), Self::make_ident_entry(vr, vals));
    }

    // --- RIGHT SIDE HELPERS (DICOM → DICOMweb) ---

    fn set_response(
        envelope: &mut RequestEnvelope<Value>,
        status: http::StatusCode,
        headers: HashMap<String, String>,
        body: Option<String>,
        body_b64: Option<String>,
        json: Option<Value>,
    ) {
        // Build response meta map
        let mut resp = serde_json::Map::new();
        resp.insert("status".to_string(), serde_json::json!(status.as_u16()));
        if !headers.is_empty() {
            resp.insert("headers".to_string(), serde_json::json!(headers));
        }
        if let Some(s) = body {
            resp.insert("body".to_string(), serde_json::json!(s));
        }
        if let Some(b64) = body_b64 {
            resp.insert("body_b64".to_string(), serde_json::json!(b64));
        }
        if let Some(j) = json {
            resp.insert("json".to_string(), j);
        }

        // Merge into existing normalized_data and mirror into original_data so the
        // pipeline's right-side conversion preserves our response for the endpoint.
        let nd = envelope
            .normalized_data
            .take()
            .unwrap_or_else(|| serde_json::json!({}));
        let mut map = nd.as_object().cloned().unwrap_or_default();
        map.insert("response".to_string(), Value::Object(resp));
        let new_nd = Value::Object(map);
        envelope.normalized_data = Some(new_nd.clone());
        envelope.original_data = new_nd;
    }

    fn build_qido_json_from_matches(matches: &Value) -> Value {
        // Convert identifier JSON objects to full DICOMweb JSON objects
        match matches {
            Value::Array(arr) => {
                let mut dicomweb_objects = Vec::new();
                for item in arr {
                    // Each item should be a DICOM identifier JSON from the backend
                    // Convert it to full DICOMweb format
                    if let Ok(dicom_obj) = dicom_json_tool::json_value_to_identifier(item) {
                        if let Ok(full_json) = dicom_json_tool::identifier_to_json_value(&dicom_obj)
                        {
                            dicomweb_objects.push(full_json);
                        } else {
                            // Fallback: use the identifier as-is
                            dicomweb_objects.push(item.clone());
                        }
                    } else {
                        // Fallback: use the identifier as-is
                        dicomweb_objects.push(item.clone());
                    }
                }
                Value::Array(dicomweb_objects)
            }
            other => {
                // Single object case
                if let Ok(dicom_obj) = dicom_json_tool::json_value_to_identifier(other) {
                    if let Ok(full_json) = dicom_json_tool::identifier_to_json_value(&dicom_obj) {
                        Value::Array(vec![full_json])
                    } else {
                        Value::Array(vec![other.clone()])
                    }
                } else {
                    Value::Array(vec![other.clone()])
                }
            }
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
        // Common tag mappings
        // StudyInstanceUID -> 0020000D (UI)
        // SeriesInstanceUID -> 0020000E (UI)
        // SOPInstanceUID -> 00080018 (UI)
        // PatientID -> 00100020 (LO)
        // AccessionNumber -> 00080050 (SH)
        // Modality -> 00080060 (CS)
        let mut ident = serde_json::Map::<String, Value>::new();

        // Pull selected QIDO query params
        if let Some(v) = Self::qp_first(qp, "StudyInstanceUID") {
            Self::add_tag(&mut ident, "0020000D", "UI", vec![v]);
        }
        if let Some(v) = Self::qp_first(qp, "SeriesInstanceUID") {
            Self::add_tag(&mut ident, "0020000E", "UI", vec![v]);
        }
        if let Some(v) = Self::qp_first(qp, "SOPInstanceUID") {
            Self::add_tag(&mut ident, "00080018", "UI", vec![v]);
        }
        if let Some(v) = Self::qp_first(qp, "PatientID") {
            Self::add_tag(&mut ident, "00100020", "LO", vec![v]);
        }
        if let Some(v) = Self::qp_first(qp, "AccessionNumber") {
            Self::add_tag(&mut ident, "00080050", "SH", vec![v]);
        }
        if let Some(v) = Self::qp_first(qp, "Modality") {
            Self::add_tag(&mut ident, "00080060", "CS", vec![v]);
        }

        // Route-based mapping
        let mut op = None::<&str>;
        match parts.as_slice() {
            // QIDO: /studies
            ["studies"] => {
                op = Some("find");
                // Include return keys for common attributes by using empty Value arrays
                ident
                    .entry("0020000D")
                    .or_insert_with(|| Self::make_ident_entry("UI", vec![])); // StudyInstanceUID
                ident
                    .entry("00080020")
                    .or_insert_with(|| Self::make_ident_entry("DA", vec![])); // StudyDate
                ident
                    .entry("00080061")
                    .or_insert_with(|| Self::make_ident_entry("CS", vec![])); // ModalitiesInStudy
                ident
                    .entry("00100020")
                    .or_insert_with(|| Self::make_ident_entry("LO", vec![])); // PatientID
                ident
                    .entry("00100010")
                    .or_insert_with(|| Self::make_ident_entry("PN", vec![])); // PatientName
            }
            // QIDO: /studies/{study}/series
            ["studies", study_uid, "series"] => {
                op = Some("find");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                ident
                    .entry("0020000E")
                    .or_insert_with(|| Self::make_ident_entry("UI", vec![])); // SeriesInstanceUID return key
                ident
                    .entry("00080060")
                    .or_insert_with(|| Self::make_ident_entry("CS", vec![])); // Modality
            }
            // QIDO: /studies/{study}/series/{series}/instances
            ["studies", study_uid, "series", series_uid, "instances"] => {
                op = Some("find");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                Self::add_tag(
                    &mut ident,
                    "0020000E",
                    "UI",
                    vec![(*series_uid).to_string()],
                );
                ident
                    .entry("00080018")
                    .or_insert_with(|| Self::make_ident_entry("UI", vec![])); // SOPInstanceUID return key
            }
            // WADO: /studies/{study}/series/{series}/instances/{instance}
            ["studies", study_uid, "series", series_uid, "instances", instance_uid] => {
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

        // QIDO lists -> application/dicom+json
        if operation == "find" {
            let matches_val = nd.get("matches").cloned().unwrap_or(Value::Array(vec![]));
            let json = Self::build_qido_json_from_matches(&matches_val);
            let mut hdrs = HashMap::new();
            hdrs.insert(
                "content-type".to_string(),
                "application/dicom+json".to_string(),
            );

            // Return 200 if we have results, 204 if empty
            let status = if json.as_array().is_some_and(|arr| !arr.is_empty()) {
                http::StatusCode::OK
            } else {
                http::StatusCode::NO_CONTENT
            };

            Self::set_response(&mut envelope, status, hdrs, None, None, Some(json));
            return Ok(envelope);
        }

        // WADO metadata -> application/dicom+json
        if path.ends_with("/metadata") {
            // Prefer instances array; otherwise matches
            let datasets = nd.get("instances").or_else(|| nd.get("matches"));
            let json = datasets.cloned().unwrap_or(Value::Array(vec![]));
            let mut hdrs = HashMap::new();
            hdrs.insert(
                "content-type".to_string(),
                "application/dicom+json".to_string(),
            );
            Self::set_response(
                &mut envelope,
                http::StatusCode::OK,
                hdrs,
                None,
                None,
                Some(json),
            );
            return Ok(envelope);
        }

        // WADO instance retrieval -> multipart/related if we have folder_path
        if operation == "get" && path.contains("/instances/") && !path.contains("/frames/") {
            if let Some(folder_path) = nd.get("folder_path").and_then(|v| v.as_str()) {
                match Self::read_instance_bytes(folder_path) {
                    Ok(parts) => {
                        let (boundary, body_bytes) = Self::build_multipart(parts);
                        let mut hdrs = HashMap::new();
                        hdrs.insert(
                            "content-type".to_string(),
                            format!(
                                "multipart/related; type=\"application/dicom\"; boundary={}",
                                boundary
                            ),
                        );
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&body_bytes);
                        Self::set_response(
                            &mut envelope,
                            http::StatusCode::OK,
                            hdrs,
                            None,
                            Some(b64),
                            None,
                        );
                        return Ok(envelope);
                    }
                    Err(_e) => {
                        // fall through and do nothing; let endpoint serialize default JSON
                    }
                }
            }
        }

        // WADO frames -> decode and encode to image/jpeg or image/png
        if operation == "get" && (path.contains("/frames/") || path.contains("frames/")) {
            // Accept negotiation
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
                                let mut hdrs = HashMap::new();
                                hdrs.insert("content-type".to_string(), content_type.to_string());
                                let b64 =
                                    base64::engine::general_purpose::STANDARD.encode(&images[0]);
                                Self::set_response(
                                    &mut envelope,
                                    http::StatusCode::OK,
                                    hdrs,
                                    None,
                                    Some(b64),
                                    None,
                                );
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
                                let mut hdrs = HashMap::new();
                                hdrs.insert(
                                    "content-type".to_string(),
                                    format!(
                                        "multipart/related; type=\"{}\"; boundary={}",
                                        content_type, boundary
                                    ),
                                );
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&body);
                                Self::set_response(
                                    &mut envelope,
                                    http::StatusCode::OK,
                                    hdrs,
                                    None,
                                    Some(b64),
                                    None,
                                );
                                return Ok(envelope);
                            }
                        }
                        Err(_e) => {
                            // Unsupported TS or decoding error
                            let mut hdrs = HashMap::new();
                            hdrs.insert("content-type".to_string(), "application/json".to_string());
                            let problem = serde_json::json!({
                                "error": "UnsupportedTransferSyntax",
                                "message": "Unable to decode frames for requested instance",
                            });
                            Self::set_response(
                                &mut envelope,
                                http::StatusCode::NOT_ACCEPTABLE,
                                hdrs,
                                None,
                                None,
                                Some(problem),
                            );
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
