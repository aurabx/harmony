use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;

/// Middleware that maps DICOMweb HTTP requests (QIDO/WADO) into DIMSE operations
/// for the DICOM backend. It converts path + query to a DICOM identifier JSON and
/// sets metadata.path to one of: "find", "get".
///
/// Behavior:
/// - Runs on the LEFT (incoming) chain.
/// - Clears any earlier endpoint response (e.g., DICOMweb endpoint skeleton 501) and removes skip_backends.
/// - Produces identifier JSON under normalized_data.dimse_identifier for the DICOM backend to consume.
#[derive(Default, Debug)]
pub struct DicomwebToDicomMiddleware;

impl DicomwebToDicomMiddleware {
    pub fn new() -> Self {
        Self::default()
    }

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
}

#[async_trait::async_trait]
impl Middleware for DicomwebToDicomMiddleware {
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
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // No-op on right side; responses are handled by backend and endpoint
        Ok(envelope)
    }
}
