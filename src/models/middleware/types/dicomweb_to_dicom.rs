use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::models::middleware::UpstreamFailure;
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
pub struct DICOMwebToDICOMMiddleware;

impl DICOMwebToDICOMMiddleware {
    pub fn new() -> Self {
        Self
    }

    // --- LEFT SIDE HELPERS (DICOMweb → DICOM) ---

    fn set_backend_path(metadata: &mut HashMap<String, String>, _op: &str) {
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
        envelope: &mut ResponseEnvelope<Value>,
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
        map.insert(
            "dicomweb_response_type".to_string(),
            Value::String(response_type.to_string()),
        );
        map.insert("dicomweb_data".to_string(), data);

        // Include any additional metadata
        if let Some(meta) = metadata {
            map.insert("dicomweb_metadata".to_string(), Value::Object(meta));
        }

        let new_nd = Value::Object(map);
        envelope.normalized_data = Some(new_nd.clone());
        envelope.original_data = new_nd;
    }

    /// Build QIDO-RS JSON response from backend matches
    ///
    /// This method ensures QIDO responses are ALWAYS arrays, as required by the DICOMweb spec.
    /// Even single results are wrapped in a one-element array. This method does NOT inject
    /// missing UIDs or synthesize data - it passes through exactly what the backend provides.
    ///
    /// # Arguments
    /// * `matches` - The backend response data (array or single object)
    /// * `includefield` - Optional list of fields to include (for filtering)
    ///
    /// # Returns
    /// A JSON array containing the QIDO results
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
                // Single object case - wrap in array per DICOMweb spec
                let processed_item = Self::process_item(includefield, other);
                Value::Array(vec![processed_item])
            }
        }
    }

    fn process_item(includefield: Option<&Vec<String>>, item: &Value) -> Value {
        match includefield {
            Some(_fields) => {
                // If filtering is requested, work with the identifier as-is instead of
                // converting to full JSON first, to avoid including unwanted attributes
                Self::filter_dicom_json(item, includefield)
            }
            None => {
                // No filtering requested, convert to full DICOMweb JSON
                if let Ok(dicom_obj) = dicom_json_tool::json_value_to_identifier(item) {
                    if let Ok(full_json) = dicom_json_tool::identifier_to_json_value(&dicom_obj) {
                        full_json
                    } else {
                        item.clone()
                    }
                } else {
                    item.clone()
                }
            }
        }
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
        let add_tag_if_missing = |ident: &mut serde_json::Map<String, Value>, name: &str| {
            let tag_hex = Self::dicom_name_to_hex(name);
            // Only add if not already present (preserves query parameters)
            if !ident.contains_key(&tag_hex) {
                let vr = Self::infer_vr_for_tag(&tag_hex);
                Self::add_tag(ident, &tag_hex, &vr, vec![]);
            }
        };

        match level {
            "study" => {
                // Default study-level return keys for full metadata
                add_tag_if_missing(ident, "StudyInstanceUID");
                add_tag_if_missing(ident, "StudyDate");
                add_tag_if_missing(ident, "ModalitiesInStudy");
                add_tag_if_missing(ident, "PatientID");
                add_tag_if_missing(ident, "PatientName");
                add_tag_if_missing(ident, "StudyDescription");
                add_tag_if_missing(ident, "AccessionNumber");
            }
            "series" => {
                // Default series-level return keys
                add_tag_if_missing(ident, "SeriesInstanceUID");
                add_tag_if_missing(ident, "Modality");
                add_tag_if_missing(ident, "SeriesDescription");
                add_tag_if_missing(ident, "SeriesNumber");
            }
            "instance" => {
                // Default instance-level return keys
                add_tag_if_missing(ident, "SOPInstanceUID");
                add_tag_if_missing(ident, "InstanceNumber");
                add_tag_if_missing(ident, "SOPClassUID");
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

/// Parse the `boundary` parameter from a `multipart/related` Content-Type value.
///
/// The parameter may appear anywhere after the media type and may be quoted.
/// Returns `None` when no `boundary=` parameter is found.
/// Detects a DIMSE backend response that reports a failed C-FIND, C-GET or
/// C-MOVE.
///
/// Fires only on an explicit JSON `"success": false`. A DICOMweb HTTP backend
/// sends no `success` key, so its responses never match. STOW-RS (`"store"`)
/// has its own PS3.18 failure response and is not matched here.
///
/// Returns the operation name and the raw backend error. The error often
/// names hosts, ports and AE titles, so it is for server logs only.
pub(crate) fn dimse_operation_failure(nd: &Value) -> Option<(String, String)> {
    if nd.get("success").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    let operation = nd.get("operation").and_then(Value::as_str)?;
    if !matches!(operation, "find" | "get" | "move") {
        return None;
    }
    let detail = nd
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| nd.to_string());
    Some((operation.to_string(), detail))
}

pub(crate) fn parse_multipart_boundary(content_type: &str) -> Option<String> {
    for part in content_type.split(';') {
        let trimmed = part.trim();
        // Case-insensitive match on the parameter name only.
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("boundary=") {
            // Recover the original-case value from `trimmed`, respecting quotes.
            let raw = &trimmed[trimmed.len() - rest.len()..];
            let value = raw.trim_matches('"').trim();
            if value.is_empty() {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

/// Extract each `application/dicom` body from a `multipart/related` payload.
///
/// The bridge uses this to strip the STOW-RS multipart wrapper before forwarding
/// the DICOM payload to the DIMSE store handler. Parts whose Content-Type header
/// is not `application/dicom` are skipped. The order of the returned parts matches
/// the order they appear in the body.
///
/// Errors:
/// - The Content-Type header is missing the `boundary` parameter.
/// - The body is empty.
/// - Multipart parsing yields zero `application/dicom` parts.
pub(crate) async fn extract_multipart_dicom_parts(
    body: &[u8],
    content_type: &str,
) -> Result<Vec<Vec<u8>>, Error> {
    if body.is_empty() {
        return Err(Error::from(
            "STOW-RS multipart body is empty",
        ));
    }

    let boundary = parse_multipart_boundary(content_type).ok_or_else(|| {
        Error::from(
            "STOW-RS Content-Type has no boundary parameter",
        )
    })?;

    // multer expects a stream of Result<Bytes, _>. Feed the whole buffer as one chunk.
    use bytes::Bytes;
    use futures_util::stream;
    use multer::Multipart;

    let body_bytes = Bytes::copy_from_slice(body);
    let stream = stream::once(async move { Ok::<Bytes, std::io::Error>(body_bytes) });
    let mut multipart = Multipart::new(stream, boundary);

    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut skipped: usize = 0;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| Error::from(format!("STOW-RS multipart parse error: {}", e)))?
    {
        // multer exposes the part's Content-Type header via .content_type().
        let ct = field
            .content_type()
            .map(|m| m.essence_str().to_ascii_lowercase())
            .unwrap_or_default();

        if ct != "application/dicom" {
            skipped += 1;
            tracing::debug!(
                "STOW-RS multipart: skipping part with content-type '{}'",
                ct
            );
            // Drain the part body so multer advances to the next one.
            let _ = field
                .bytes()
                .await
                .map_err(|e| Error::from(format!("STOW-RS multipart drain error: {}", e)))?;
            continue;
        }

        let data = field
            .bytes()
            .await
            .map_err(|e| Error::from(format!("STOW-RS multipart part read error: {}", e)))?;
        parts.push(data.to_vec());
    }

    if parts.is_empty() {
        return Err(Error::from(format!(
            "STOW-RS multipart body has no application/dicom parts (skipped {} other part(s))",
            skipped
        )));
    }

    Ok(parts)
}

/// PS3.18 Failure Reason 0x0110, Processing failure. The remote node did not
/// take the instance.
pub(crate) const STOW_FAILURE_PROCESSING: u16 = 0x0110;

/// PS3.18 Failure Reason 0xC000, Cannot understand. The instance was never
/// sent because it could not be read.
pub(crate) const STOW_FAILURE_CANNOT_UNDERSTAND: u16 = 0xC000;

/// The outcome of storing one instance.
///
/// The `dicom_scu` backend returns one per instance in the `results` array of
/// its store response. `build_stow_response` turns them into the PS3.18 Store
/// Instances Response. Both sides use this type so the contract lives in one
/// place.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct StowPartResult {
    /// Position of the instance in the request, from 0.
    pub index: usize,
    /// True when the remote node took the instance.
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sop_class_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sop_instance_uid: Option<String>,
    /// PS3.18 Failure Reason. Set only when `success` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<u16>,
    /// Why the instance was not stored. Set only when `success` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl StowPartResult {
    /// The JSON form carried in the backend response.
    pub(crate) fn to_json(&self) -> Value {
        // Serialising can only fail for maps with non-string keys or a custom
        // `Serialize` that errors. This struct has neither.
        serde_json::to_value(self).expect("StowPartResult always serialises to JSON")
    }
}

/// Build the PS3.18 STOW-RS Store Instances Response from a store's results.
///
/// `results` is the `results` array of a `dicom_scu` store response, one
/// `StowPartResult` per instance. Returns the HTTP status and the response
/// data set in the DICOM JSON model (PS3.18 Annex F.2):
/// - 200 when every instance was stored, 202 when some were, 409 when none were.
/// - Referenced SOP Sequence (0008,1199) lists the stored instances.
/// - Failed SOP Sequence (0008,1198) lists failed instances whose SOP Class
///   UID and SOP Instance UID are both known.
/// - Other Failures Sequence (0008,119A) holds the Failure Reason of every
///   other failure.
///
/// An empty `results` array stored nothing, so it gives 409 with one Other
/// Failures item of 0xC000. An entry that does not parse is counted as a
/// failure with Failure Reason 0x0110.
pub(crate) fn build_stow_response(results: &[Value]) -> (u16, Value) {
    let mut stored: usize = 0;
    let mut referenced: Vec<Value> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    let mut other: Vec<Value> = Vec::new();

    for (position, raw) in results.iter().enumerate() {
        let entry = match serde_json::from_value::<StowPartResult>(raw.clone()) {
            Ok(entry) => entry,
            Err(e) => {
                // The entry may hold UIDs and backend error text. Log neither.
                tracing::error!(
                    "STOW-RS: unreadable store result at position {}: {}",
                    position,
                    e
                );
                other.push(stow_other_failure_item(STOW_FAILURE_PROCESSING));
                continue;
            }
        };
        let class_uid = entry.sop_class_uid.filter(|u| !u.is_empty());
        let instance_uid = entry.sop_instance_uid.filter(|u| !u.is_empty());
        if entry.success {
            stored += 1;
            match (class_uid, instance_uid) {
                (Some(class_uid), Some(instance_uid)) => {
                    referenced.push(stow_referenced_item(class_uid, instance_uid))
                }
                // Both UIDs are Type 1 in a Referenced SOP item, so an item
                // without them would be invalid. Listing the instance as a
                // failure would be false: the PACS has it. The instance still
                // counts as stored for the status code, and is left out of the
                // sequence. The SCU cannot send an instance without both UIDs,
                // so this only happens when Harmony could not read them back.
                (_, _) => tracing::warn!(
                    "STOW-RS: instance {} was stored but its SOP UIDs are unknown; \
                     it is not listed in the Referenced SOP Sequence",
                    entry.index
                ),
            }
            continue;
        }
        let reason = entry.failure_reason.unwrap_or(STOW_FAILURE_PROCESSING);
        match (class_uid, instance_uid) {
            (Some(class_uid), Some(instance_uid)) => {
                failed.push(stow_failed_item(class_uid, instance_uid, reason))
            }
            _ => other.push(stow_other_failure_item(reason)),
        }
    }
    if results.is_empty() {
        other.push(stow_other_failure_item(STOW_FAILURE_CANNOT_UNDERSTAND));
    }

    let any_failed = !failed.is_empty() || !other.is_empty();
    let status = match (stored == 0, any_failed) {
        (false, false) => 200,
        (false, true) => 202,
        (true, _) => 409,
    };

    let mut dataset = serde_json::Map::new();
    dataset.insert("00081190".to_string(), stow_empty_retrieve_url());
    if !referenced.is_empty() {
        dataset.insert(
            "00081199".to_string(),
            json!({"vr": "SQ", "Value": referenced}),
        );
    }
    if !failed.is_empty() {
        dataset.insert("00081198".to_string(), json!({"vr": "SQ", "Value": failed}));
    }
    if !other.is_empty() {
        dataset.insert("0008119A".to_string(), json!({"vr": "SQ", "Value": other}));
    }
    (status, Value::Object(dataset))
}

/// Retrieve URL (0008,1190) is Type 2 and is always sent empty. Harmony cannot
/// reliably know the public scheme, host and path prefix a client used, for
/// example behind a reverse proxy. Absolute URLs are a follow-up.
fn stow_empty_retrieve_url() -> Value {
    json!({"vr": "UR"})
}

fn stow_referenced_item(class_uid: String, instance_uid: String) -> Value {
    json!({
        "00081150": {"vr": "UI", "Value": [class_uid]},
        "00081155": {"vr": "UI", "Value": [instance_uid]},
        "00081190": stow_empty_retrieve_url(),
    })
}

fn stow_failed_item(class_uid: String, instance_uid: String, reason: u16) -> Value {
    json!({
        "00081150": {"vr": "UI", "Value": [class_uid]},
        "00081155": {"vr": "UI", "Value": [instance_uid]},
        "00081197": {"vr": "US", "Value": [reason]},
    })
}

fn stow_other_failure_item(reason: u16) -> Value {
    json!({"00081197": {"vr": "US", "Value": [reason]}})
}

#[async_trait::async_trait]
impl Middleware for DICOMwebToDICOMMiddleware {
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
        // DELETE requests pass through to the backend without DIMSE translation
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

        // Extract pagination parameters for QIDO-RS queries
        // Per DICOMweb spec: limit controls maximum results, offset controls skip count
        let limit = qp
            .get("limit")
            .and_then(|v| v.first())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(100); // Default limit of 100 results

        let offset = qp
            .get("offset")
            .and_then(|v| v.first())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        // Store pagination parameters in metadata for right-side processing
        envelope
            .request_details
            .metadata
            .insert("dicomweb_limit".to_string(), limit.to_string());
        envelope
            .request_details
            .metadata
            .insert("dicomweb_offset".to_string(), offset.to_string());

        tracing::debug!("DICOMweb pagination: limit={}, offset={}", limit, offset);

        // Process all query parameters (except special ones like includefield/limit/offset)
        for (param_name, param_values) in qp {
            // Skip special DICOMweb parameters that aren't DICOM tags
            if matches!(
                param_name.as_str(),
                "includefield" | "limit" | "offset" | "fuzzymatching"
            ) {
                continue;
            }

            // Convert parameter name to DICOM hex tag
            let tag_hex = Self::dicom_name_to_hex(param_name);
            let vr = Self::infer_vr_for_tag(&tag_hex);

            // Use all values for this parameter (DICOMweb allows multiple values)
            // Date ranges in DICOM format (YYYYMMDD-YYYYMMDD) are passed through as-is
            let values: Vec<String> = param_values.to_vec();
            if !values.is_empty() {
                Self::add_tag(&mut ident, &tag_hex, &vr, values);
            }
        }

        // Parse includefield query parameter for attribute filtering
        // DICOMweb spec allows comma-separated values in a single parameter value
        let includefield: Option<Vec<String>> = qp.get("includefield").map(|values| {
            values
                .iter()
                .flat_map(|s| s.split(',').map(|t| t.trim().to_string()))
                .filter(|t| !t.is_empty())
                .collect()
        });

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
            // QIDO: /series (top-level series search)
            ["series"] => {
                op = Some("find");
                add_return_keys(&mut ident, "series");
            }
            // QIDO: /instances (top-level instance search)
            ["instances"] => {
                op = Some("find");
                add_return_keys(&mut ident, "instance");
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
                Self::add_tag(
                    &mut ident,
                    "0020000D",
                    &study_vr,
                    vec![(*study_uid).to_string()],
                );
                Self::add_tag(
                    &mut ident,
                    "0020000E",
                    &series_vr,
                    vec![(*series_uid).to_string()],
                );
                add_return_keys(&mut ident, "series");
            }
            // QIDO: /studies/{study}/series/{series}/instances
            ["studies", study_uid, "series", series_uid, "instances"] => {
                op = Some("find");
                let study_vr = Self::infer_vr_for_tag("0020000D");
                let series_vr = Self::infer_vr_for_tag("0020000E");
                Self::add_tag(
                    &mut ident,
                    "0020000D",
                    &study_vr,
                    vec![(*study_uid).to_string()],
                );
                Self::add_tag(
                    &mut ident,
                    "0020000E",
                    &series_vr,
                    vec![(*series_uid).to_string()],
                );
                add_return_keys(&mut ident, "instance");
            }
            // Handle /studies/{study}/series/{series}/instances/{instance} for both QIDO and WADO
            ["studies", study_uid, "series", series_uid, "instances", instance_uid] => {
                // Check Accept header to determine if this is WADO instance retrieval
                let accept = envelope
                    .request_details
                    .headers
                    .get("accept")
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();

                // WADO instance retrieval if Accept header requests binary DICOM or multipart
                // Distinguish between application/dicom (binary) and application/dicom+json (JSON)
                let is_wado_retrieval = (accept.contains("application/dicom")
                    && !accept.contains("application/dicom+json"))
                    || accept.contains("multipart/related");

                if is_wado_retrieval {
                    // For WADO instance retrieval (binary DICOM data)
                    op = Some("get");
                    let study_vr = Self::infer_vr_for_tag("0020000D");
                    let series_vr = Self::infer_vr_for_tag("0020000E");
                    let instance_vr = Self::infer_vr_for_tag("00080018");
                    Self::add_tag(
                        &mut ident,
                        "0020000D",
                        &study_vr,
                        vec![(*study_uid).to_string()],
                    );
                    Self::add_tag(
                        &mut ident,
                        "0020000E",
                        &series_vr,
                        vec![(*series_uid).to_string()],
                    );
                    Self::add_tag(
                        &mut ident,
                        "00080018",
                        &instance_vr,
                        vec![(*instance_uid).to_string()],
                    );
                } else {
                    // QIDO find for specific instance (JSON metadata)
                    op = Some("find");
                    let study_vr = Self::infer_vr_for_tag("0020000D");
                    let series_vr = Self::infer_vr_for_tag("0020000E");
                    let instance_vr = Self::infer_vr_for_tag("00080018");
                    Self::add_tag(
                        &mut ident,
                        "0020000D",
                        &study_vr,
                        vec![(*study_uid).to_string()],
                    );
                    Self::add_tag(
                        &mut ident,
                        "0020000E",
                        &series_vr,
                        vec![(*series_uid).to_string()],
                    );
                    Self::add_tag(
                        &mut ident,
                        "00080018",
                        &instance_vr,
                        vec![(*instance_uid).to_string()],
                    );
                    add_return_keys(&mut ident, "instance");
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
            // WADO rendered: /studies/{study}/rendered
            ["studies", study_uid, "rendered"] => {
                op = Some("get");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                envelope.request_details.metadata.insert(
                    "dicomweb_response_mode".to_string(),
                    "rendered".to_string(),
                );
            }
            // WADO rendered: /studies/{study}/series/{series}/rendered
            ["studies", study_uid, "series", series_uid, "rendered"] => {
                op = Some("get");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                Self::add_tag(&mut ident, "0020000E", "UI", vec![(*series_uid).to_string()]);
                envelope.request_details.metadata.insert(
                    "dicomweb_response_mode".to_string(),
                    "rendered".to_string(),
                );
            }
            // WADO rendered: /studies/{study}/series/{series}/instances/{instance}/rendered
            ["studies", study_uid, "series", series_uid, "instances", instance_uid, "rendered"] => {
                op = Some("get");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                Self::add_tag(&mut ident, "0020000E", "UI", vec![(*series_uid).to_string()]);
                Self::add_tag(&mut ident, "00080018", "UI", vec![(*instance_uid).to_string()]);
                envelope.request_details.metadata.insert(
                    "dicomweb_response_mode".to_string(),
                    "rendered".to_string(),
                );
            }
            // WADO thumbnail: /studies/{study}/thumbnail
            ["studies", study_uid, "thumbnail"] => {
                op = Some("get");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                envelope.request_details.metadata.insert(
                    "dicomweb_response_mode".to_string(),
                    "thumbnail".to_string(),
                );
            }
            // WADO thumbnail: /studies/{study}/series/{series}/thumbnail
            ["studies", study_uid, "series", series_uid, "thumbnail"] => {
                op = Some("get");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                Self::add_tag(&mut ident, "0020000E", "UI", vec![(*series_uid).to_string()]);
                envelope.request_details.metadata.insert(
                    "dicomweb_response_mode".to_string(),
                    "thumbnail".to_string(),
                );
            }
            // WADO thumbnail: /studies/{study}/series/{series}/instances/{instance}/thumbnail
            ["studies", study_uid, "series", series_uid, "instances", instance_uid, "thumbnail"] => {
                op = Some("get");
                Self::add_tag(&mut ident, "0020000D", "UI", vec![(*study_uid).to_string()]);
                Self::add_tag(&mut ident, "0020000E", "UI", vec![(*series_uid).to_string()]);
                Self::add_tag(&mut ident, "00080018", "UI", vec![(*instance_uid).to_string()]);
                envelope.request_details.metadata.insert(
                    "dicomweb_response_mode".to_string(),
                    "thumbnail".to_string(),
                );
            }
            // WADO bulkdata (map to get; actual subresource ignored for now)
            ["bulkdata", ..] => {
                op = Some("get");
            }
            _ => {}
        }

        if let Some(op_name) = op {
            // Ensure metadata prepared for backend (legacy path)
            Self::set_backend_path(&mut envelope.request_details.metadata, op_name);

            // Also set the typed Command for modern consumers
            use crate::models::envelope::command::{Command, Operation};
            let operation = match op_name {
                "find" => Operation::Query,
                "get" => Operation::Retrieve,
                "move" => Operation::Move,
                "store" => Operation::Store,
                _ => Operation::Retrieve, // Default for unrecognized ops
            };
            envelope.command = Some(Command::new(operation).with_source("dicomweb_to_dicom_middleware"));

            // Clear any response set by upstream endpoint
            let mut nd = envelope
                .normalized_data
                .clone()
                .unwrap_or_else(|| json!({}));
            Self::clear_endpoint_response(&mut nd);

            // Attach identifier for backend to consume
            if let Some(obj) = nd.as_object_mut() {
                obj.insert("dimse_identifier".to_string(), Value::Object(ident));
                // For QIDO-RS (find) operations, pass limit to DIMSE backend
                // Note: offset will be applied in right() since DIMSE doesn't support it natively
                if op_name == "find" {
                    obj.insert(
                        "max_results".to_string(),
                        Value::Number((limit + offset).into()),
                    );
                }
            } else {
                let mut map = serde_json::Map::new();
                map.insert("dimse_identifier".to_string(), Value::Object(ident));
                if op_name == "find" {
                    map.insert(
                        "max_results".to_string(),
                        Value::Number((limit + offset).into()),
                    );
                }
                nd = Value::Object(map);
            }
            envelope.normalized_data = Some(nd);
        }

        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
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

        tracing::debug!(
            full_path = ?envelope.request_details.metadata.get("full_path"),
            path_meta = ?envelope.request_details.metadata.get("path"),
            raw_path = %raw_path,
            "dicomweb_bridge.right: incoming path analysis"
        );

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

        // Resolve the operation from the typed Command on the response envelope.
        // The Command is set by endpoints or left-side middleware and propagated
        // through the backend via from_backend_with_command().
        use crate::models::envelope::command::Operation;
        let resolved_op = envelope.resolve_operation();
        let success = nd.get("success").and_then(|v| v.as_bool()).unwrap_or(false);

        tracing::debug!(
            dicomweb_subpath = %path,
            operation = ?resolved_op,
            success = %success,
            has_matches = nd.get("matches").is_some(),
            has_instances = nd.get("instances").is_some(),
            "dicomweb_bridge.right: backend response analysis"
        );

        // Parse includefield from request metadata (set by left-side middleware)
        let includefield: Option<Vec<String>> = envelope
            .request_details
            .metadata
            .get("dicomweb_includefield")
            .and_then(|json_str| serde_json::from_str(json_str).ok());

        // Parse pagination parameters from request metadata (set by left-side middleware)
        let limit = envelope
            .request_details
            .metadata
            .get("dicomweb_limit")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100);

        let offset = envelope
            .request_details
            .metadata
            .get("dicomweb_offset")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);

        // A failed DIMSE find/get/move must not reach the client as an empty
        // result or as the raw backend JSON. The typed error lets each
        // adapter answer with a protocol status and no detail.
        if let Some((operation, detail)) = dimse_operation_failure(&nd) {
            tracing::error!(
                operation = %operation,
                dicomweb_subpath = %path,
                error = %detail,
                "dicomweb_bridge.right: DIMSE backend reported a failure"
            );
            return Err(Box::new(UpstreamFailure { operation, detail }));
        }

        // QIDO lists -> DICOMweb JSON data
        if resolved_op == Some(Operation::Query) {
            let matches_val = nd.get("matches").cloned().unwrap_or(Value::Array(vec![]));
            let mut json = Self::build_qido_json_from_matches(&matches_val, includefield.as_ref());

            // Apply pagination to QIDO-RS results
            // Offset is applied here since DIMSE C-FIND doesn't natively support it
            if let Value::Array(ref mut results) = json {
                let total_results = results.len();
                tracing::debug!(
                    total_results = total_results,
                    limit = limit,
                    offset = offset,
                    "Applying pagination to QIDO-RS results"
                );

                // Skip offset items and take limit items
                let start = offset.min(total_results);
                let end = (start + limit).min(total_results);
                *results = results.drain(start..end).collect();

                tracing::debug!(
                    paginated_count = results.len(),
                    "Pagination applied, returning {} results",
                    results.len()
                );
            }

            // Create metadata indicating whether results were found
            let has_results = match &json {
                Value::Array(arr) => !arr.is_empty(),
                _ => true, // Non-array responses are considered to have results
            };

            tracing::debug!(
                response_is_array = json.is_array(),
                result_count = json.as_array().map(|a| a.len()).unwrap_or(0),
                has_results = %has_results,
                "dicomweb_bridge.right: QIDO response shaped"
            );

            let mut metadata = serde_json::Map::new();
            metadata.insert("has_results".to_string(), Value::Bool(has_results));

            Self::set_dicomweb_data(&mut envelope, "qido_json", json, Some(metadata));
            return Ok(envelope);
        }

        // STOW-RS through a DIMSE backend -> PS3.18 Store Instances Response.
        // A DICOMweb HTTP backend answers in DICOMweb already and has no
        // `results` array, so its response passes through untouched.
        if resolved_op == Some(Operation::Store)
            && nd.get("operation").and_then(|v| v.as_str()) == Some("store")
        {
            if let Some(results) = nd.get("results").and_then(|v| v.as_array()) {
                let (status, dataset) = build_stow_response(results);
                tracing::debug!(
                    status = status,
                    parts = results.len(),
                    "dicomweb_bridge.right: STOW-RS response shaped"
                );
                let mut metadata = serde_json::Map::new();
                metadata.insert("status".to_string(), Value::from(status));
                Self::set_dicomweb_data(&mut envelope, "stow_json", dataset, Some(metadata));
                return Ok(envelope);
            }
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

        // Rendered/thumbnail passthrough -> pass image data from backend unchanged
        // This must be checked BEFORE the WADO instance branch to avoid incorrectly
        // matching instance-level rendered/thumbnail paths (e.g., /studies/.../instances/{uid}/rendered)
        if matches!(resolved_op, Some(Operation::Retrieve | Operation::Move))
            && (path.ends_with("/rendered") || path.ends_with("/thumbnail"))
        {
            // For Pipeline 2 (DIMSE backend): the backend returns raw DICOM objects, not images.
            // Server-side rendering is not yet implemented — the response passes through as-is
            // and the endpoint will return whatever the backend provided.
            //
            // For Pipeline 1 (DICOMweb HTTP backend): the upstream dicom_to_dicomweb middleware
            // already recognizes image/* content-types and marks them as image_passthrough.
            // The response body and headers are preserved on the envelope, so the DICOMweb
            // endpoint can return the image directly in the HTTP response via standard handling.
            tracing::debug!(
                "dicomweb_bridge.right: rendered/thumbnail route, passing through: {}",
                path
            );
            return Ok(envelope);
        }

        // WADO instance retrieval -> multipart DICOM data
        if matches!(resolved_op, Some(Operation::Retrieve | Operation::Move)) && path.contains("/instances/") && !path.contains("/frames/") {
            if let Some(folder_path) = nd.get("folder_path").and_then(|v| v.as_str()) {
                match Self::read_instance_bytes(folder_path) {
                    Ok(parts) => {
                        let (boundary, body_bytes) = Self::build_multipart(parts);
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&body_bytes);

                        let mut metadata = serde_json::Map::new();
                        metadata.insert("boundary".to_string(), Value::String(boundary));
                        metadata.insert("body_b64".to_string(), Value::String(b64));

                        Self::set_dicomweb_data(
                            &mut envelope,
                            "wado_instance",
                            Value::Null,
                            Some(metadata),
                        );
                        return Ok(envelope);
                    }
                    Err(e) => {
                        // Falling through would send the backend JSON, which
                        // holds the server-side folder path, to the client.
                        tracing::error!(
                            dicomweb_subpath = %path,
                            folder_path = %folder_path,
                            error = %e,
                            "dicomweb_bridge.right: could not read retrieved instances"
                        );
                        return Err(Error::from(format!(
                            "dicomweb_bridge: could not read retrieved instances: {}",
                            e
                        )));
                    }
                }
            }
        }

        // WADO frames -> decode and encode image data
        if matches!(resolved_op, Some(Operation::Retrieve | Operation::Move)) && (path.contains("/frames/") || path.contains("frames/")) {
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
                                let b64 =
                                    base64::engine::general_purpose::STANDARD.encode(&images[0]);
                                let mut metadata = serde_json::Map::new();
                                metadata.insert(
                                    "content_type".to_string(),
                                    Value::String(content_type.to_string()),
                                );
                                metadata.insert("body_b64".to_string(), Value::String(b64));
                                metadata.insert("is_single_frame".to_string(), Value::Bool(true));

                                Self::set_dicomweb_data(
                                    &mut envelope,
                                    "wado_frames",
                                    Value::Null,
                                    Some(metadata),
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
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&body);

                                let mut metadata = serde_json::Map::new();
                                metadata.insert(
                                    "content_type".to_string(),
                                    Value::String(content_type.to_string()),
                                );
                                metadata.insert("boundary".to_string(), Value::String(boundary));
                                metadata.insert("body_b64".to_string(), Value::String(b64));
                                metadata.insert("is_single_frame".to_string(), Value::Bool(false));

                                Self::set_dicomweb_data(
                                    &mut envelope,
                                    "wado_frames",
                                    Value::Null,
                                    Some(metadata),
                                );
                                return Ok(envelope);
                            }
                        }
                        Err(_e) => {
                            // Unsupported TS or decoding error - let endpoint handle the error response
                            let mut metadata = serde_json::Map::new();
                            metadata.insert(
                                "error".to_string(),
                                Value::String("UnsupportedTransferSyntax".to_string()),
                            );
                            metadata.insert(
                                "message".to_string(),
                                Value::String(
                                    "Unable to decode frames for requested instance".to_string(),
                                ),
                            );

                            Self::set_dicomweb_data(
                                &mut envelope,
                                "wado_frames_error",
                                Value::Null,
                                Some(metadata),
                            );
                            return Ok(envelope);
                        }
                    }
                }
            }
        }

        // Handle backend failures for frame and instance requests
        if !success {
            if path.contains("/frames/") {
                // The raw backend error can name internal hosts, so it goes
                // to the server log and the client gets a fixed message.
                tracing::error!(
                    dicomweb_subpath = %path,
                    error = ?nd.get("error"),
                    "dicomweb_bridge.right: frame retrieval failed"
                );

                let mut metadata = serde_json::Map::new();
                metadata.insert(
                    "error".to_string(),
                    Value::String("FrameRetrievalFailed".to_string()),
                );
                metadata.insert(
                    "message".to_string(),
                    Value::String("Frame retrieval failed".to_string()),
                );

                Self::set_dicomweb_data(
                    &mut envelope,
                    "wado_frames_error",
                    Value::Null,
                    Some(metadata),
                );
                return Ok(envelope);
            }
            // For other failed requests, let the original response pass through
            return Ok(envelope);
        }

        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{RequestDetails, RequestEnvelopeBuilder};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_left_includefield_adds_return_keys() {
        let bridge = DICOMwebToDICOMMiddleware::new();

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

        let envelope = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/api/dicom/studies")
            .query_params(query_params)
            .metadata_entry("path", "studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        // Process through the middleware
        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed_envelope = result.unwrap();
        
        // Verify that command is set for backend processing (check before consuming normalized_data)
        use crate::models::envelope::command::Operation;
        assert_eq!(
            processed_envelope.resolve_operation(),
            Some(Operation::Query)
        );
        
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

        // Verify skip_backends is set to false
        assert_eq!(
            processed_envelope
                .request_details
                .metadata
                .get("skip_backends"),
            Some(&"false".to_string())
        );
    }

    #[tokio::test]
    async fn test_left_no_includefield_uses_defaults() {
        let bridge = DICOMwebToDICOMMiddleware::new();

        // Create a mock request envelope without includefield
        let envelope = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/api/dicom/studies")
            .metadata_entry("path", "studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        // Process through the middleware
        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed_envelope = result.unwrap();
        
        // Verify that command is set for backend processing (check before consuming normalized_data)
        use crate::models::envelope::command::Operation;
        assert_eq!(
            processed_envelope.resolve_operation(),
            Some(Operation::Query)
        );
        
        // Verify no includefield metadata is stored
        assert!(!processed_envelope
            .request_details
            .metadata
            .contains_key("dicomweb_includefield"));
        
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
    }

    #[tokio::test]
    async fn test_left_includefield_preserves_search_criteria() {
        let bridge = DICOMwebToDICOMMiddleware::new();

        // Create a mock request with both search params and includefield
        let mut query_params: HashMap<String, Vec<String>> = HashMap::new();
        query_params.insert("PatientID".to_string(), vec!["12345".to_string()]);
        query_params.insert(
            "includefield".to_string(),
            vec!["PatientID".to_string(), "PatientName".to_string()],
        );

        let envelope = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/api/dicom/studies")
            .query_params(query_params)
            .metadata_entry("path", "studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

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
        use crate::models::envelope::command::{Command, Operation};
        let bridge = DICOMwebToDICOMMiddleware::new();

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
            content_metadata: None,
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
            "success": true,
            "matches": matches
        });

        let envelope = ResponseEnvelope {
            request_details,
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            command: Some(Command::new(Operation::Query)),
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
            metadata
                .unwrap()
                .get("has_results")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_left_processes_all_query_parameters() {
        let bridge = DICOMwebToDICOMMiddleware::new();

        // Create a mock request with various DICOM query parameters
        let mut query_params: HashMap<String, Vec<String>> = HashMap::new();
        query_params.insert("PatientName".to_string(), vec!["Doe^John".to_string()]);
        query_params.insert("StudyDate".to_string(), vec!["20231015".to_string()]);
        query_params.insert(
            "Modality".to_string(),
            vec!["CT".to_string(), "MR".to_string()],
        ); // Multiple values
        query_params.insert(
            "StudyDescription".to_string(),
            vec!["Brain Study".to_string()],
        );
        query_params.insert("SeriesNumber".to_string(), vec!["1".to_string()]);
        query_params.insert("includefield".to_string(), vec!["PatientName".to_string()]); // Should be skipped
        query_params.insert("limit".to_string(), vec!["100".to_string()]); // Should be skipped

        let envelope = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/dicomweb/studies")
            .query_params(query_params)
            .metadata_entry("path", "studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        // Process through the middleware
        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed_envelope = result.unwrap();
        let nd = processed_envelope.normalized_data.unwrap();
        let identifier = nd.get("dimse_identifier").unwrap().as_object().unwrap();

        // Verify all query parameters were processed
        assert!(
            identifier.contains_key("00100010"),
            "PatientName should be present"
        );
        assert!(
            identifier.contains_key("00080020"),
            "StudyDate should be present"
        );
        assert!(
            identifier.contains_key("00080060"),
            "Modality should be present"
        );
        assert!(
            identifier.contains_key("00081030"),
            "StudyDescription should be present"
        );
        assert!(
            identifier.contains_key("00200011"),
            "SeriesNumber should be present"
        );

        // Verify multiple values are preserved
        let modality_values = identifier
            .get("00080060")
            .and_then(|m| m.get("Value"))
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(modality_values.len(), 2);
        assert!(modality_values.contains(&serde_json::json!("CT")));
        assert!(modality_values.contains(&serde_json::json!("MR")));

        // Verify special parameters were skipped (not present as DICOM tags)
        // These would have nonsensical hex representations if processed
        let all_keys: Vec<&String> = identifier.keys().collect();
        assert!(
            !all_keys.iter().any(|k| k.starts_with("include")),
            "includefield should not be processed as DICOM tag"
        );
        assert!(
            !all_keys.iter().any(|k| k.starts_with("limit")),
            "limit should not be processed as DICOM tag"
        );
    }

    #[tokio::test]
    async fn test_pagination_default_limit() {
        let bridge = DICOMwebToDICOMMiddleware::new();

        // No limit specified - should use default of 100
        let envelope = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/dicomweb/studies")
            .metadata_entry("path", "studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();
        assert_eq!(
            processed.request_details.metadata.get("dicomweb_limit"),
            Some(&"100".to_string())
        );
        assert_eq!(
            processed.request_details.metadata.get("dicomweb_offset"),
            Some(&"0".to_string())
        );
    }

    #[tokio::test]
    async fn test_pagination_custom_limit_offset() {
        let bridge = DICOMwebToDICOMMiddleware::new();

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), vec!["25".to_string()]);
        query_params.insert("offset".to_string(), vec!["10".to_string()]);

        let envelope = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/dicomweb/studies?limit=25&offset=10")
            .query_params(query_params)
            .metadata_entry("path", "studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();
        assert_eq!(
            processed.request_details.metadata.get("dicomweb_limit"),
            Some(&"25".to_string())
        );
        assert_eq!(
            processed.request_details.metadata.get("dicomweb_offset"),
            Some(&"10".to_string())
        );

        // Verify max_results = limit + offset
        let nd = processed.normalized_data.unwrap();
        assert_eq!(nd.get("max_results").and_then(|v| v.as_u64()), Some(35));
    }

    #[tokio::test]
    async fn test_pagination_filtering_right_side() {
        use crate::models::envelope::command::{Command, Operation};
        let bridge = DICOMwebToDICOMMiddleware::new();

        // Create mock response with 10 results
        let mock_matches = serde_json::json!([
            {"00100020": {"vr": "LO", "Value": ["P001"]}},
            {"00100020": {"vr": "LO", "Value": ["P002"]}},
            {"00100020": {"vr": "LO", "Value": ["P003"]}},
            {"00100020": {"vr": "LO", "Value": ["P004"]}},
            {"00100020": {"vr": "LO", "Value": ["P005"]}},
            {"00100020": {"vr": "LO", "Value": ["P006"]}},
            {"00100020": {"vr": "LO", "Value": ["P007"]}},
            {"00100020": {"vr": "LO", "Value": ["P008"]}},
            {"00100020": {"vr": "LO", "Value": ["P009"]}},
            {"00100020": {"vr": "LO", "Value": ["P010"]}},
        ]);

        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), "studies".to_string());
        metadata.insert("full_path".to_string(), "/dicomweb/studies".to_string());
        metadata.insert("dicomweb_limit".to_string(), "3".to_string());
        metadata.insert("dicomweb_offset".to_string(), "2".to_string());

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/dicomweb/studies?limit=3&offset=2".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            content_metadata: None,
            metadata,
        };

        let normalized_data = serde_json::json!({
            "success": true,
            "matches": mock_matches
        });

        let envelope = ResponseEnvelope {
            request_details,
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            command: Some(Command::new(Operation::Query)),
            original_data: serde_json::json!({}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };

        let result = bridge.right(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();
        let nd = processed.normalized_data.unwrap();

        // Verify pagination was applied: offset=2, limit=3 -> indices 2,3,4 (P003, P004, P005)
        let data = nd.get("dicomweb_data").and_then(|v| v.as_array());
        assert!(data.is_some());
        let data_array = data.unwrap();
        assert_eq!(data_array.len(), 3);

        // Verify correct results
        assert_eq!(
            data_array[0]
                .get("00100020")
                .and_then(|v| v.get("Value"))
                .and_then(|v| v.get(0))
                .and_then(|v| v.as_str()),
            Some("P003")
        );
    }

    #[tokio::test]
    async fn test_date_range_query_passed_through() {
        let bridge = DICOMwebToDICOMMiddleware::new();

        let mut query_params = HashMap::new();
        query_params.insert(
            "StudyDate".to_string(),
            vec!["20240101-20240131".to_string()],
        );

        let envelope = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/dicomweb/studies?StudyDate=20240101-20240131")
            .query_params(query_params)
            .metadata_entry("path", "studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();
        let nd = processed.normalized_data.unwrap();
        let identifier = nd.get("dimse_identifier").unwrap().as_object().unwrap();

        // Verify StudyDate tag is present with date range
        assert!(
            identifier.contains_key("00080020"),
            "StudyDate tag should be present"
        );
        let study_date = identifier.get("00080020").unwrap();
        let values = study_date.get("Value").and_then(|v| v.as_array());
        assert!(values.is_some(), "StudyDate should have Value array");
        let values = values.unwrap();
        assert!(
            !values.is_empty(),
            "StudyDate Value array should not be empty"
        );
        assert_eq!(values[0].as_str(), Some("20240101-20240131"));
    }

    #[tokio::test]
    async fn test_combined_pagination_and_date_range() {
        let bridge = DICOMwebToDICOMMiddleware::new();

        let mut query_params = HashMap::new();
        query_params.insert(
            "StudyDate".to_string(),
            vec!["20240101-20240131".to_string()],
        );
        query_params.insert("limit".to_string(), vec!["10".to_string()]);
        query_params.insert("offset".to_string(), vec!["5".to_string()]);

        let envelope = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/dicomweb/studies?StudyDate=20240101-20240131&limit=10&offset=5")
            .query_params(query_params)
            .metadata_entry("path", "studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        let result = bridge.left(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();

        // Verify pagination parameters
        assert_eq!(
            processed.request_details.metadata.get("dicomweb_limit"),
            Some(&"10".to_string())
        );
        assert_eq!(
            processed.request_details.metadata.get("dicomweb_offset"),
            Some(&"5".to_string())
        );

        // Verify date range is in identifier
        let nd = processed.normalized_data.unwrap();
        let identifier = nd.get("dimse_identifier").unwrap().as_object().unwrap();
        assert!(identifier.contains_key("00080020"));

        // Verify max_results = limit + offset
        assert_eq!(nd.get("max_results").and_then(|v| v.as_u64()), Some(15));
    }

    // ==================== Rendered/Thumbnail Passthrough Tests ====================

    #[tokio::test]
    async fn test_right_rendered_study_passes_through() {
        use crate::models::envelope::command::{Command, Operation};
        let bridge = DICOMwebToDICOMMiddleware::new();

        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), "studies/1.2.3/rendered".to_string());
        metadata.insert(
            "full_path".to_string(),
            "/dicomweb/studies/1.2.3/rendered".to_string(),
        );

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/dicomweb/studies/1.2.3/rendered".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            content_metadata: None,
            metadata,
        };

        // Simulate a backend response with image data
        let normalized_data = serde_json::json!({
            "success": true,
            "some_image_data": "preserved"
        });

        let envelope = ResponseEnvelope {
            request_details,
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: {
                    let mut h = HashMap::new();
                    h.insert("content-type".to_string(), "image/jpeg".to_string());
                    h
                },
                metadata: HashMap::new(),
            },
            command: Some(Command::new(Operation::Retrieve)),
            original_data: serde_json::json!({}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };

        let result = bridge.right(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();

        // Verify the envelope passed through unmodified — no dicomweb_response_type set
        let nd = processed.normalized_data.unwrap();
        assert!(
            nd.get("dicomweb_response_type").is_none(),
            "Rendered passthrough should not set dicomweb_response_type"
        );
        // Verify original data is preserved
        assert_eq!(
            nd.get("some_image_data").and_then(|v| v.as_str()),
            Some("preserved")
        );
        // Verify content-type header is preserved
        assert_eq!(
            processed.response_details.headers.get("content-type"),
            Some(&"image/jpeg".to_string())
        );
    }

    #[tokio::test]
    async fn test_right_thumbnail_instance_passes_through() {
        use crate::models::envelope::command::{Command, Operation};
        let bridge = DICOMwebToDICOMMiddleware::new();

        let mut metadata = HashMap::new();
        metadata.insert(
            "path".to_string(),
            "studies/1.2.3/series/4.5.6/instances/7.8.9/thumbnail".to_string(),
        );
        metadata.insert(
            "full_path".to_string(),
            "/dicomweb/studies/1.2.3/series/4.5.6/instances/7.8.9/thumbnail".to_string(),
        );

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/dicomweb/studies/1.2.3/series/4.5.6/instances/7.8.9/thumbnail".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            content_metadata: None,
            metadata,
        };

        let normalized_data = serde_json::json!({
            "success": true
        });

        let envelope = ResponseEnvelope {
            request_details,
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: {
                    let mut h = HashMap::new();
                    h.insert("content-type".to_string(), "image/png".to_string());
                    h
                },
                metadata: HashMap::new(),
            },
            command: Some(Command::new(Operation::Retrieve)),
            original_data: serde_json::json!({}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };

        let result = bridge.right(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();

        // Verify the envelope passed through — no dicomweb_response_type, no mangling
        let nd = processed.normalized_data.unwrap();
        assert!(
            nd.get("dicomweb_response_type").is_none(),
            "Thumbnail passthrough should not set dicomweb_response_type"
        );
        assert_eq!(
            processed.response_details.headers.get("content-type"),
            Some(&"image/png".to_string())
        );
    }

    #[tokio::test]
    async fn test_right_instance_rendered_not_treated_as_wado_instance() {
        // Regression test: ensure instance-level rendered path is NOT matched by the
        // WADO instance retrieval branch (which would try to read DICOM files from folder_path)
        use crate::models::envelope::command::{Command, Operation};
        let bridge = DICOMwebToDICOMMiddleware::new();

        let mut metadata = HashMap::new();
        metadata.insert(
            "path".to_string(),
            "studies/1.2.3/series/4.5.6/instances/7.8.9/rendered".to_string(),
        );
        metadata.insert(
            "full_path".to_string(),
            "/dicomweb/studies/1.2.3/series/4.5.6/instances/7.8.9/rendered".to_string(),
        );

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/dicomweb/studies/1.2.3/series/4.5.6/instances/7.8.9/rendered".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            content_metadata: None,
            metadata,
        };

        let normalized_data = serde_json::json!({
            "success": true,
            "folder_path": "/nonexistent/path"
        });

        let envelope = ResponseEnvelope {
            request_details,
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: {
                    let mut h = HashMap::new();
                    h.insert("content-type".to_string(), "image/jpeg".to_string());
                    h
                },
                metadata: HashMap::new(),
            },
            command: Some(Command::new(Operation::Retrieve)),
            original_data: serde_json::json!({}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };

        let result = bridge.right(envelope).await;
        assert!(result.is_ok());

        let processed = result.unwrap();

        // Should NOT have tried to read from folder_path and build multipart.
        // If it had, it would have set dicomweb_response_type to "wado_instance".
        let nd = processed.normalized_data.unwrap();
        assert!(
            nd.get("dicomweb_response_type").is_none(),
            "Instance-level rendered should NOT be treated as WADO instance retrieval"
        );
    }

    // ==================== End-to-end Command Propagation Tests ====================

    #[tokio::test]
    async fn test_command_propagation_left_to_right_query() {
        // Verifies: request → left() sets Command → simulated backend → right() dispatches on Command
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();

        // 1. Build a QIDO-RS request (studies search)
        let mut query_params = HashMap::new();
        query_params.insert("PatientName".to_string(), vec!["Doe^*".to_string()]);

        let request = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/dicomweb/studies")
            .query_params(query_params)
            .metadata_entry("path", "studies")
            .metadata_entry("full_path", "/dicomweb/studies")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        // 2. Pass through left() — should set Command::Query
        let after_left = bridge.left(request).await.unwrap();
        assert_eq!(
            after_left.resolve_operation(),
            Some(Operation::Query),
            "left() should set Command to Query for QIDO-RS"
        );

        // 3. Simulate backend: create ResponseEnvelope propagating the command
        let response = ResponseEnvelope {
            request_details: after_left.request_details.clone(),
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            command: after_left.command.clone(),
            original_data: serde_json::json!({}),
            normalized_data: Some(serde_json::json!({
                "success": true,
                "matches": [
                    {
                        "00100010": {"vr": "PN", "Value": ["Doe^John"]},
                        "0020000D": {"vr": "UI", "Value": ["1.2.3.4.5"]}
                    }
                ]
            })),
            normalized_snapshot: None,
        };

        // 4. Pass through right() — should dispatch on Command to produce QIDO response
        let after_right = bridge.right(response).await.unwrap();

        // 5. Verify Command is preserved on the response
        assert_eq!(
            after_right.resolve_operation(),
            Some(Operation::Query),
            "Command should be preserved through the right() side"
        );

        // 6. Verify QIDO response was generated (Command-driven dispatch worked)
        let nd = after_right.normalized_data.unwrap();
        assert_eq!(
            nd.get("dicomweb_response_type").and_then(|v| v.as_str()),
            Some("qido_json"),
            "right() should produce QIDO JSON response when Command is Query"
        );
        assert!(nd.get("dicomweb_data").is_some());
    }

    #[tokio::test]
    async fn test_command_propagation_left_to_right_retrieve_rendered() {
        // Verifies: WADO-RS rendered → left() sets Command::Retrieve →
        //           right() dispatches on Command+path to pass through rendered response
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();

        // 1. Build a WADO-RS rendered request
        let request = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/dicomweb/studies/1.2.3/rendered")
            .metadata_entry("path", "studies/1.2.3/rendered")
            .metadata_entry("full_path", "/dicomweb/studies/1.2.3/rendered")
            .original_data(serde_json::json!({}))
            .build()
            .unwrap();

        // 2. Pass through left() — should set Command::Retrieve
        let after_left = bridge.left(request).await.unwrap();
        assert_eq!(
            after_left.resolve_operation(),
            Some(Operation::Retrieve),
            "left() should set Command to Retrieve for WADO-RS rendered"
        );

        // 3. Simulate backend: create ResponseEnvelope propagating the command
        let response = ResponseEnvelope {
            request_details: after_left.request_details.clone(),
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: {
                    let mut h = HashMap::new();
                    h.insert("content-type".to_string(), "image/jpeg".to_string());
                    h
                },
                metadata: HashMap::new(),
            },
            command: after_left.command.clone(),
            original_data: serde_json::json!({}),
            normalized_data: Some(serde_json::json!({
                "success": true,
                "image_data": "mock_rendered_content"
            })),
            normalized_snapshot: None,
        };

        // 4. Pass through right() — rendered response should pass through untransformed
        let after_right = bridge.right(response).await.unwrap();

        // 5. Verify Command is preserved through the full pipeline
        assert_eq!(
            after_right.resolve_operation(),
            Some(Operation::Retrieve),
            "Command should be preserved through the right() side"
        );

        // 6. Verify rendered response passed through (no dicomweb_response_type set)
        let nd = after_right.normalized_data.unwrap();
        assert!(
            nd.get("dicomweb_response_type").is_none(),
            "Rendered passthrough should not set dicomweb_response_type"
        );
        // Original data preserved
        assert_eq!(
            nd.get("image_data").and_then(|v| v.as_str()),
            Some("mock_rendered_content"),
            "Rendered response data should pass through unchanged"
        );
    }

    #[tokio::test]
    async fn test_right_without_command_passes_through() {
        // Verifies: if no Command is set on the response, right() does not transform
        let bridge = DICOMwebToDICOMMiddleware::new();

        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), "studies".to_string());
        metadata.insert("full_path".to_string(), "/dicomweb/studies".to_string());

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/dicomweb/studies".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            content_metadata: None,
            metadata,
        };

        let normalized_data = serde_json::json!({
            "success": true,
            "matches": [{"00100010": {"vr": "PN", "Value": ["Test"]}}]
        });

        let envelope = ResponseEnvelope {
            request_details,
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            command: None,
            original_data: serde_json::json!({}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };

        let result = bridge.right(envelope).await.unwrap();

        // Without a Command, the response should pass through unmodified
        let nd = result.normalized_data.unwrap();
        assert!(
            nd.get("dicomweb_response_type").is_none(),
            "Without a Command, right() should not transform the response"
        );
        // Original data should still be present
        assert!(nd.get("matches").is_some());
    }

    // ==================== STOW-RS multipart parsing tests ====================

    /// Build a multipart/related body from a list of (content-type, body) parts.
    fn build_multipart_related_body(boundary: &str, parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        for (ct, body) in parts {
            buf.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            buf.extend_from_slice(format!("Content-Type: {}\r\n\r\n", ct).as_bytes());
            buf.extend_from_slice(body);
            buf.extend_from_slice(b"\r\n");
        }
        buf.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
        buf
    }

    #[test]
    fn test_parse_multipart_boundary_quoted() {
        let ct = "multipart/related; type=\"application/dicom\"; boundary=\"abc123\"";
        assert_eq!(
            parse_multipart_boundary(ct).as_deref(),
            Some("abc123"),
            "quoted boundary should be extracted without quotes"
        );
    }

    #[test]
    fn test_parse_multipart_boundary_unquoted() {
        let ct = "multipart/related; boundary=simpleboundary";
        assert_eq!(
            parse_multipart_boundary(ct).as_deref(),
            Some("simpleboundary")
        );
    }

    #[test]
    fn test_parse_multipart_boundary_absent() {
        let ct = "multipart/related; type=\"application/dicom\"";
        assert!(
            parse_multipart_boundary(ct).is_none(),
            "no boundary parameter must yield None"
        );
    }

    #[tokio::test]
    async fn test_extract_multipart_dicom_parts_single_part() {
        let boundary = "harmonytest";
        let dicom_bytes: &[u8] = b"\x00\x01\x02FAKE-DICOM-PAYLOAD-1\x03\x04";
        let body = build_multipart_related_body(
            boundary,
            &[("application/dicom", dicom_bytes)],
        );
        let ct = format!(
            "multipart/related; type=\"application/dicom\"; boundary={}",
            boundary
        );

        let parts = extract_multipart_dicom_parts(&body, &ct)
            .await
            .expect("single part must parse");

        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].as_slice(), dicom_bytes);
    }

    #[tokio::test]
    async fn test_extract_multipart_dicom_parts_two_parts_preserves_order() {
        let boundary = "harmonytest";
        let first: &[u8] = b"FIRST-DICOM-PART";
        let second: &[u8] = b"SECOND-DICOM-PART-with-more-bytes";
        let body = build_multipart_related_body(
            boundary,
            &[
                ("application/dicom", first),
                ("application/dicom", second),
            ],
        );
        let ct = format!("multipart/related; boundary={}", boundary);

        let parts = extract_multipart_dicom_parts(&body, &ct)
            .await
            .expect("two parts must parse");

        assert_eq!(parts.len(), 2, "both parts must be extracted");
        assert_eq!(parts[0].as_slice(), first, "first part must come first");
        assert_eq!(parts[1].as_slice(), second, "second part must come second");
    }

    #[tokio::test]
    async fn test_extract_multipart_dicom_parts_missing_boundary_rejected() {
        let ct = "multipart/related; type=\"application/dicom\"";
        let body: &[u8] = b"anything";

        let err = extract_multipart_dicom_parts(body, ct)
            .await
            .expect_err("missing boundary must return an error");

        let msg = format!("{}", err);
        assert!(
            msg.contains("boundary"),
            "error should name the missing boundary parameter, got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_extract_multipart_dicom_parts_empty_body_rejected() {
        let ct = "multipart/related; boundary=harmonytest";
        let body: &[u8] = b"";

        let err = extract_multipart_dicom_parts(body, ct)
            .await
            .expect_err("empty body must return an error");

        let msg = format!("{}", err);
        assert!(
            msg.contains("empty"),
            "error should say the body is empty, got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_extract_multipart_dicom_parts_skips_non_dicom_parts() {
        let boundary = "harmonytest";
        let json_part: &[u8] = br#"{"unexpected":"metadata"}"#;
        let dicom_bytes: &[u8] = b"REAL-DICOM-PAYLOAD";
        let body = build_multipart_related_body(
            boundary,
            &[
                ("application/json", json_part),
                ("application/dicom", dicom_bytes),
            ],
        );
        let ct = format!("multipart/related; boundary={}", boundary);

        let parts = extract_multipart_dicom_parts(&body, &ct)
            .await
            .expect("skipping non-dicom part should still succeed");

        assert_eq!(parts.len(), 1, "only the application/dicom part is kept");
        assert_eq!(parts[0].as_slice(), dicom_bytes);
    }

    // ==================== STOW-RS response tests ====================

    const CT_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.2";

    fn stored(instance_uid: &str) -> Value {
        serde_json::json!({
            "index": 0,
            "success": true,
            "sop_class_uid": CT_IMAGE_STORAGE,
            "sop_instance_uid": instance_uid
        })
    }

    fn not_stored(instance_uid: &str) -> Value {
        serde_json::json!({
            "index": 1,
            "success": false,
            "sop_class_uid": CT_IMAGE_STORAGE,
            "sop_instance_uid": instance_uid,
            "failure_reason": 0x0110,
            "error": "C-STORE failed"
        })
    }

    fn unreadable() -> Value {
        serde_json::json!({
            "index": 2,
            "success": false,
            "failure_reason": 0xC000,
            "error": "part is not a DICOM Part 10 file"
        })
    }

    /// The items of the sequence `tag`, after checking its VR.
    fn sequence_items<'a>(dataset: &'a Value, tag: &str) -> &'a Vec<Value> {
        assert_eq!(dataset[tag]["vr"], "SQ", "{} in {}", tag, dataset);
        dataset[tag]["Value"]
            .as_array()
            .unwrap_or_else(|| panic!("{} has no items in {}", tag, dataset))
    }

    fn assert_empty_retrieve_url(item: &Value) {
        assert_eq!(
            item["00081190"],
            serde_json::json!({"vr": "UR"}),
            "in {}",
            item
        );
    }

    #[test]
    fn test_stow_response_all_stored_is_200() {
        let (status, dataset) = build_stow_response(&[stored("1.2.3.1"), stored("1.2.3.2")]);

        assert_eq!(status, 200);
        assert_empty_retrieve_url(&dataset);
        let items = sequence_items(&dataset, "00081199");
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0]["00081150"],
            serde_json::json!({"vr": "UI", "Value": [CT_IMAGE_STORAGE]})
        );
        assert_eq!(
            items[0]["00081155"],
            serde_json::json!({"vr": "UI", "Value": ["1.2.3.1"]})
        );
        assert_empty_retrieve_url(&items[0]);
        assert!(
            dataset.get("00081198").is_none(),
            "no Failed SOP Sequence: {}",
            dataset
        );
        assert!(
            dataset.get("0008119A").is_none(),
            "no Other Failures: {}",
            dataset
        );
    }

    #[test]
    fn test_stow_response_some_stored_is_202() {
        let (status, dataset) = build_stow_response(&[stored("1.2.3.1"), not_stored("1.2.3.2")]);

        assert_eq!(status, 202);
        assert_eq!(sequence_items(&dataset, "00081199").len(), 1);
        let failed = sequence_items(&dataset, "00081198");
        assert_eq!(failed.len(), 1);
        assert_eq!(
            failed[0]["00081155"],
            serde_json::json!({"vr": "UI", "Value": ["1.2.3.2"]})
        );
        assert!(
            dataset.get("0008119A").is_none(),
            "no Other Failures: {}",
            dataset
        );
    }

    #[test]
    fn test_stow_response_none_stored_is_409() {
        let (status, dataset) = build_stow_response(&[not_stored("1.2.3.2")]);

        assert_eq!(status, 409);
        let failed = sequence_items(&dataset, "00081198");
        assert_eq!(
            failed[0]["00081150"],
            serde_json::json!({"vr": "UI", "Value": [CT_IMAGE_STORAGE]})
        );
        // US values are JSON numbers. 0x0110 is 272.
        assert_eq!(
            failed[0]["00081197"],
            serde_json::json!({"vr": "US", "Value": [272]})
        );
        assert!(failed[0].get("00081190").is_none(), "{}", failed[0]);
        assert!(
            dataset.get("00081199").is_none(),
            "no Referenced SOP: {}",
            dataset
        );
        assert!(
            dataset.get("0008119A").is_none(),
            "no Other Failures: {}",
            dataset
        );
        assert_empty_retrieve_url(&dataset);
    }

    #[test]
    fn test_stow_response_unreadable_part_is_an_other_failure() {
        let (status, dataset) = build_stow_response(&[stored("1.2.3.1"), unreadable()]);

        assert_eq!(status, 202);
        let other = sequence_items(&dataset, "0008119A");
        // 0xC000 is 49152.
        assert_eq!(
            other,
            &vec![serde_json::json!({"00081197": {"vr": "US", "Value": [49152]}})]
        );
        assert!(
            dataset.get("00081198").is_none(),
            "no Failed SOP: {}",
            dataset
        );
    }

    #[test]
    fn test_stow_response_without_results_is_409() {
        let (status, dataset) = build_stow_response(&[]);

        assert_eq!(status, 409);
        assert_eq!(
            sequence_items(&dataset, "0008119A"),
            &vec![serde_json::json!({"00081197": {"vr": "US", "Value": [49152]}})]
        );
        assert!(
            dataset.get("00081199").is_none(),
            "no Referenced SOP: {}",
            dataset
        );
        assert!(
            dataset.get("00081198").is_none(),
            "no Failed SOP: {}",
            dataset
        );
    }

    #[test]
    fn test_stow_response_unparseable_result_is_a_processing_failure() {
        let (status, dataset) = build_stow_response(&[serde_json::json!({"index": 0})]);

        assert_eq!(status, 409);
        assert_eq!(
            sequence_items(&dataset, "0008119A"),
            &vec![serde_json::json!({"00081197": {"vr": "US", "Value": [272]}})]
        );
    }

    #[test]
    fn test_stow_response_stored_without_uids_counts_but_is_not_referenced() {
        let no_uids = serde_json::json!({"index": 1, "success": true});

        let (status, dataset) = build_stow_response(&[stored("1.2.3.1"), no_uids.clone()]);

        // Stored, so 200 and not a failure. Only the identifiable one is listed.
        assert_eq!(status, 200);
        let items = sequence_items(&dataset, "00081199");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0]["00081155"],
            serde_json::json!({"vr": "UI", "Value": ["1.2.3.1"]})
        );
        assert!(
            dataset.get("00081198").is_none(),
            "no Failed SOP: {}",
            dataset
        );
        assert!(
            dataset.get("0008119A").is_none(),
            "no Other Failures: {}",
            dataset
        );

        let (status, dataset) = build_stow_response(&[no_uids]);
        assert_eq!(status, 200);
        assert!(
            dataset.get("00081199").is_none(),
            "no invalid item: {}",
            dataset
        );
    }

    #[test]
    fn test_stow_part_result_round_trips_through_json() {
        let result = StowPartResult {
            index: 3,
            success: false,
            sop_class_uid: Some(CT_IMAGE_STORAGE.to_string()),
            sop_instance_uid: None,
            failure_reason: Some(STOW_FAILURE_PROCESSING),
            error: Some("refused".to_string()),
        };

        let json = result.to_json();

        assert!(
            json.get("sop_instance_uid").is_none(),
            "None is left out: {}",
            json
        );
        assert_eq!(json["failure_reason"], 272);
        assert_eq!(
            serde_json::from_value::<StowPartResult>(json).unwrap(),
            result
        );
    }

    fn store_response_envelope(normalized_data: Value) -> ResponseEnvelope<Value> {
        use crate::models::envelope::command::{Command, Operation};
        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), "studies".to_string());
        metadata.insert("full_path".to_string(), "/dicomweb/studies".to_string());
        ResponseEnvelope {
            request_details: RequestDetails {
                method: "POST".to_string(),
                uri: "/dicomweb/studies".to_string(),
                headers: HashMap::new(),
                cookies: HashMap::new(),
                query_params: HashMap::new(),
                cache_status: None,
                content_metadata: None,
                metadata,
            },
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            command: Some(Command::new(Operation::Store)),
            original_data: serde_json::json!({}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        }
    }

    #[tokio::test]
    async fn test_right_store_builds_stow_response() {
        let bridge = DICOMwebToDICOMMiddleware::new();
        let envelope = store_response_envelope(serde_json::json!({
            "operation": "store",
            "success": false,
            "results": [stored("1.2.3.1"), not_stored("1.2.3.2")]
        }));

        let nd = bridge
            .right(envelope)
            .await
            .unwrap()
            .normalized_data
            .unwrap();

        assert_eq!(nd["dicomweb_response_type"], "stow_json");
        assert_eq!(nd["dicomweb_metadata"]["status"], 202);
        assert_eq!(sequence_items(&nd["dicomweb_data"], "00081199").len(), 1);
        assert_eq!(sequence_items(&nd["dicomweb_data"], "00081198").len(), 1);
    }

    #[tokio::test]
    async fn test_right_store_from_dicomweb_backend_passes_through() {
        // A DICOMweb HTTP backend already answers with a PS3.18 response.
        let bridge = DICOMwebToDICOMMiddleware::new();
        let upstream = serde_json::json!({
            "00081190": {"vr": "UR", "Value": ["https://pacs.example/studies/1.2.3"]},
            "00081199": {"vr": "SQ", "Value": []}
        });
        let envelope = store_response_envelope(upstream.clone());

        let nd = bridge
            .right(envelope)
            .await
            .unwrap()
            .normalized_data
            .unwrap();

        assert_eq!(nd, upstream);
    }

    /// Builds a backend response for a GET on `full_path` with the given operation.
    fn retrieve_response_envelope(
        full_path: &str,
        operation: crate::models::envelope::command::Operation,
        normalized_data: Value,
    ) -> ResponseEnvelope<Value> {
        use crate::models::envelope::command::Command;
        let mut metadata = HashMap::new();
        metadata.insert(
            "path".to_string(),
            full_path.trim_start_matches("/dicomweb/").to_string(),
        );
        metadata.insert("full_path".to_string(), full_path.to_string());
        ResponseEnvelope {
            request_details: RequestDetails {
                method: "GET".to_string(),
                uri: full_path.to_string(),
                headers: HashMap::new(),
                cookies: HashMap::new(),
                query_params: HashMap::new(),
                cache_status: None,
                content_metadata: None,
                metadata,
            },
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            command: Some(Command::new(operation)),
            original_data: serde_json::json!({}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        }
    }

    const RAW_SCU_ERROR: &str = "association to PACS_AET at 10.20.30.40:11112 refused";
    const INSTANCE_PATH: &str = "/dicomweb/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5";

    fn failed(operation: &str) -> Value {
        serde_json::json!({
            "operation": operation,
            "success": false,
            "error": RAW_SCU_ERROR
        })
    }

    fn expect_upstream_failure(result: Result<ResponseEnvelope<Value>, Error>, operation: &str) {
        let err = match result {
            Ok(envelope) => panic!(
                "expected UpstreamFailure, got Ok with {:?}",
                envelope.normalized_data
            ),
            Err(e) => e,
        };
        let failure = err
            .downcast_ref::<UpstreamFailure>()
            .expect("error must downcast to UpstreamFailure");
        assert_eq!(failure.operation, operation);
        assert_eq!(failure.detail, RAW_SCU_ERROR);
    }

    #[test]
    fn test_dimse_operation_failure_matches_failed_find_get_move() {
        for op in ["find", "get", "move"] {
            assert_eq!(
                dimse_operation_failure(&failed(op)),
                Some((op.to_string(), RAW_SCU_ERROR.to_string()))
            );
        }
    }

    #[test]
    fn test_dimse_operation_failure_ignores_store_echo_and_success() {
        assert_eq!(dimse_operation_failure(&failed("store")), None);
        assert_eq!(dimse_operation_failure(&failed("echo")), None);
        assert_eq!(
            dimse_operation_failure(&serde_json::json!({"operation": "find", "success": true})),
            None
        );
    }

    #[test]
    fn test_dimse_operation_failure_needs_explicit_false() {
        // A DICOMweb HTTP backend sends no `success` key.
        assert_eq!(
            dimse_operation_failure(&serde_json::json!({"operation": "get", "error": "x"})),
            None
        );
        // A string "false" is not a JSON bool.
        assert_eq!(
            dimse_operation_failure(&serde_json::json!({"operation": "get", "success": "false"})),
            None
        );
        // No operation means it is not a DIMSE find/get/move response.
        assert_eq!(
            dimse_operation_failure(&serde_json::json!({"success": false, "error": "x"})),
            None
        );
    }

    #[test]
    fn test_dimse_operation_failure_without_error_text_uses_whole_response() {
        let nd = serde_json::json!({"operation": "find", "success": false, "host": "pacs"});
        let (_, detail) = dimse_operation_failure(&nd).unwrap();
        assert!(detail.contains("\"host\":\"pacs\""));
    }

    #[tokio::test]
    async fn test_right_failed_find_returns_upstream_failure_not_empty_list() {
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();
        let envelope =
            retrieve_response_envelope("/dicomweb/studies", Operation::Query, failed("find"));

        expect_upstream_failure(bridge.right(envelope).await, "find");
    }

    #[tokio::test]
    async fn test_right_failed_get_on_instance_returns_upstream_failure() {
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();
        let envelope =
            retrieve_response_envelope(INSTANCE_PATH, Operation::Retrieve, failed("get"));

        expect_upstream_failure(bridge.right(envelope).await, "get");
    }

    #[tokio::test]
    async fn test_right_failed_move_returns_upstream_failure() {
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();
        let envelope = retrieve_response_envelope(INSTANCE_PATH, Operation::Move, failed("move"));

        expect_upstream_failure(bridge.right(envelope).await, "move");
    }

    #[tokio::test]
    async fn test_right_failed_get_on_metadata_returns_upstream_failure() {
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();
        let path = format!("{}/metadata", INSTANCE_PATH);
        let envelope = retrieve_response_envelope(&path, Operation::Retrieve, failed("get"));

        expect_upstream_failure(bridge.right(envelope).await, "get");
    }

    #[tokio::test]
    async fn test_right_retrieve_without_success_key_passes_through() {
        // A DICOMweb HTTP backend has no `success` key and must pass untouched.
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();
        let upstream = serde_json::json!({"operation": "get", "body": "opaque"});
        let envelope =
            retrieve_response_envelope(INSTANCE_PATH, Operation::Retrieve, upstream.clone());

        let nd = bridge
            .right(envelope)
            .await
            .unwrap()
            .normalized_data
            .unwrap();

        assert_eq!(nd, upstream);
    }

    #[tokio::test]
    async fn test_right_frames_failure_hides_raw_error() {
        // No find/get/move operation, so the frames failure branch handles it.
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();
        let path = format!("{}/frames/1", INSTANCE_PATH);
        let envelope = retrieve_response_envelope(
            &path,
            Operation::Retrieve,
            serde_json::json!({"success": false, "error": RAW_SCU_ERROR}),
        );

        let nd = bridge
            .right(envelope)
            .await
            .unwrap()
            .normalized_data
            .unwrap();

        assert_eq!(nd["dicomweb_response_type"], "wado_frames_error");
        assert_eq!(nd["dicomweb_metadata"]["message"], "Frame retrieval failed");
        // The DICOMweb endpoint builds the 406 body from dicomweb_metadata only.
        let body = nd["dicomweb_metadata"].to_string();
        assert!(!body.contains("10.20.30.40"), "raw error leaked: {}", body);
        assert!(!body.contains("PACS_AET"), "raw error leaked: {}", body);
    }

    #[tokio::test]
    async fn test_right_unreadable_instance_folder_is_an_error() {
        use crate::models::envelope::command::Operation;
        let bridge = DICOMwebToDICOMMiddleware::new();
        let envelope = retrieve_response_envelope(
            INSTANCE_PATH,
            Operation::Retrieve,
            serde_json::json!({
                "operation": "get",
                "success": true,
                "folder_path": "./tmp/aura-2566-folder-that-does-not-exist"
            }),
        );

        let err = match bridge.right(envelope).await {
            Ok(envelope) => panic!(
                "backend JSON with folder_path must not pass through: {:?}",
                envelope.normalized_data
            ),
            Err(e) => e,
        };
        assert!(err.downcast_ref::<UpstreamFailure>().is_none());
    }
}
