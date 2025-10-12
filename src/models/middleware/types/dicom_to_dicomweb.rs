use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use base64::Engine;
use dicom_pixeldata::image as img;
use dicom_pixeldata::PixelDecoder;
use img::ImageEncoder;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Right-side middleware: shapes DIMSE backend outputs into DICOMweb HTTP responses
/// Supports:
/// - QIDO-RS list responses as application/dicom+json
/// - WADO-RS instance retrieval as multipart/related (application/dicom parts) when files are on filesystem
/// - WADO-RS metadata as application/dicom+json using identifier JSON (minimal)
#[derive(Default, Debug)]
pub struct DicomToDicomwebMiddleware;

impl DicomToDicomwebMiddleware {
    pub fn new() -> Self {
        Self::default()
    }

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

    fn build_qido_json(matches: &Value) -> Value {
        // DICOMweb QIDO JSON is an array of DICOM JSON datasets
        match matches {
            Value::Array(arr) => Value::Array(arr.clone()),
            other => Value::Array(vec![other.clone()]),
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
impl Middleware for DicomToDicomwebMiddleware {
    async fn left(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // no-op
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
            let json = Self::build_qido_json(&matches_val);
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
                let instance_uid = parts.get(5).map(|s| *s).unwrap_or("");
                let frames_str = parts.get(7).map(|s| *s).unwrap_or("");
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
