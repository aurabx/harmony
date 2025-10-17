use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::services::services::{ServiceHandler, ServiceType};
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use std::sync::OnceLock;
use tracing::debug;

#[derive(Debug, Deserialize)]
pub struct MockDicomEndpoint;

/// Mock DICOM data for testing
#[derive(Debug, Clone)]
pub struct MockDicomData {
    pub study_uid: String,
    pub patient_id: String,
    pub series: Vec<MockSeries>,
}

#[derive(Debug, Clone)]
pub struct MockSeries {
    pub series_uid: String,
    pub series_number: i32,
    pub modality: String,
    pub series_description: String,
    pub instances: Vec<MockInstance>,
}

#[derive(Debug, Clone)]
pub struct MockInstance {
    pub instance_uid: String,
    pub instance_number: i32,
    pub sop_class_uid: String,
}

/// Global mock data store
static MOCK_DATA: OnceLock<MockDicomData> = OnceLock::new();

impl MockDicomData {
    /// Initialize mock data from sample DICOM UIDs
    pub fn init_sample_data() -> Self {
        Self {
            study_uid: "1.2.826.0.1.3680043.9.7133.3280065491876470".to_string(),
            patient_id: "PID156695".to_string(),
            series: vec![
                MockSeries {
                    series_uid: "1.2.826.0.1.3680043.9.7133.1734441961856038".to_string(),
                    series_number: 1,
                    modality: "CT".to_string(),
                    series_description: "Series 1".to_string(),
                    instances: vec![
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.2677554575065585".to_string(),
                            instance_number: 1,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(), // CT Image Storage
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.1494401914668643".to_string(),
                            instance_number: 2,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.1578071133979400".to_string(),
                            instance_number: 3,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.2004958842253129".to_string(),
                            instance_number: 4,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.2456165115170247".to_string(),
                            instance_number: 5,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                    ],
                },
                MockSeries {
                    series_uid: "1.2.826.0.1.3680043.9.7133.2369827755580483".to_string(),
                    series_number: 2,
                    modality: "CT".to_string(),
                    series_description: "Series 2".to_string(),
                    instances: vec![
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.3195857377419654".to_string(),
                            instance_number: 1,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.9552313113374485".to_string(),
                            instance_number: 2,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.2140212232603664".to_string(),
                            instance_number: 3,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.2298005625221305".to_string(),
                            instance_number: 4,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.2683978666224479".to_string(),
                            instance_number: 5,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                    ],
                },
                MockSeries {
                    series_uid: "1.2.826.0.1.3680043.9.7133.1326436931051844".to_string(),
                    series_number: 3,
                    modality: "CT".to_string(),
                    series_description: "Series 3".to_string(),
                    instances: vec![
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.7886317424438469".to_string(),
                            instance_number: 1,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.2847621385214465".to_string(),
                            instance_number: 2,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.2459625263340059".to_string(),
                            instance_number: 3,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.1040499691138526".to_string(),
                            instance_number: 4,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                        MockInstance {
                            instance_uid: "1.2.826.0.1.3680043.9.7133.3208456549831196".to_string(),
                            instance_number: 5,
                            sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                        },
                    ],
                },
            ],
        }
    }

    /// Get mock data instance (initialize if needed)
    pub fn instance() -> &'static Self {
        MOCK_DATA.get_or_init(Self::init_sample_data)
    }

    /// Handle a mock C-FIND request
    pub fn handle_find_query(&self, params: &HashMap<String, String>) -> Vec<serde_json::Value> {
        debug!("[MOCK DICOM] C-FIND query params: {:?}", params);

        // Determine query level based on present parameters
        let query_level = if params.get("00080018").is_some_and(|v| !v.is_empty()) {
            // SOPInstanceUID filter present -> IMAGE level
            "IMAGE"
        } else if params.contains_key("00080018")
            && (params.get("0020000D").is_some_and(|v| !v.is_empty())
                || params.get("0020000E").is_some_and(|v| !v.is_empty()))
        {
            // SOPInstanceUID return key + Study/Series filter -> query for instances (IMAGE level)
            "IMAGE"
        } else if params.get("0020000E").is_some_and(|v| !v.is_empty()) {
            // SeriesInstanceUID filter present -> SERIES level
            "SERIES"
        } else if params.contains_key("0020000E")
            && params.get("0020000D").is_some_and(|v| !v.is_empty())
        {
            // SeriesInstanceUID return key + StudyInstanceUID filter -> query for series (SERIES level)
            "SERIES"
        } else if params.get("0020000D").is_some_and(|v| !v.is_empty()) {
            // StudyInstanceUID filter present -> STUDY level (for specific study)
            "STUDY"
        } else if params.contains_key("0020000D") {
            // StudyInstanceUID return key present (but empty) -> query for studies (STUDY level)
            "STUDY"
        } else {
            // Default to PATIENT level
            "PATIENT"
        };

        debug!("[MOCK DICOM] Query level determined as: {}", query_level);

        match query_level {
            "STUDY" => self.query_studies(params),
            "SERIES" => self.query_series(params),
            "IMAGE" => self.query_instances(params),
            _ => vec![], // PATIENT level or unknown - return empty for now
        }
    }

    fn query_studies(&self, params: &HashMap<String, String>) -> Vec<serde_json::Value> {
        // Check if we're looking for a specific study
        if let Some(study_uid) = params.get("0020000D") {
            if !study_uid.is_empty() && study_uid != &self.study_uid {
                return vec![]; // Specific study not found
            }
        }

        // Check patient ID filter
        if let Some(patient_id) = params.get("00100020") {
            if !patient_id.is_empty() && patient_id != &self.patient_id {
                return vec![]; // Patient ID doesn't match
            }
        }

        // Return study-level response
        vec![serde_json::json!({
            "0020000D": {
                "vr": "UI",
                "Value": [self.study_uid]
            },
            "00100020": {
                "vr": "LO",
                "Value": [self.patient_id]
            },
            "00100010": {
                "vr": "PN",
                "Value": [{ "Alphabetic": "Doe^John" }]
            },
            "00080020": {
                "vr": "DA",
                "Value": ["20241015"]
            },
            "00080030": {
                "vr": "TM",
                "Value": ["120000"]
            },
            "00081030": {
                "vr": "LO",
                "Value": ["Mock CT Study"]
            },
            "00200010": {
                "vr": "SH",
                "Value": ["1"]
            }
        })]
    }

    fn query_series(&self, params: &HashMap<String, String>) -> Vec<serde_json::Value> {
        // Must have study UID constraint for series queries
        let study_uid = params.get("0020000D");
        if study_uid.is_none()
            || study_uid.unwrap().is_empty()
            || study_uid.unwrap() != &self.study_uid
        {
            return vec![];
        }

        // Check for specific series
        if let Some(series_uid) = params.get("0020000E") {
            if !series_uid.is_empty() {
                // Looking for specific series
                for series in &self.series {
                    if series.series_uid == *series_uid {
                        return vec![self.create_series_response(series)];
                    }
                }
                return vec![]; // Specific series not found
            }
        }

        // Check modality filter
        if let Some(modality) = params.get("00080060") {
            if !modality.is_empty() {
                // Filter by modality
                return self
                    .series
                    .iter()
                    .filter(|s| s.modality == *modality)
                    .map(|s| self.create_series_response(s))
                    .collect();
            }
        }

        // Return all series in the study
        self.series
            .iter()
            .map(|s| self.create_series_response(s))
            .collect()
    }

    fn query_instances(&self, params: &HashMap<String, String>) -> Vec<serde_json::Value> {
        // Must have study UID constraint for instance queries
        let study_uid = params.get("0020000D");
        if study_uid.is_none()
            || study_uid.unwrap().is_empty()
            || study_uid.unwrap() != &self.study_uid
        {
            return vec![];
        }

        // Must have series UID constraint for instance queries
        let series_uid = params.get("0020000E");
        if series_uid.is_none() || series_uid.unwrap().is_empty() {
            return vec![];
        }

        // Find the matching series
        let series = self
            .series
            .iter()
            .find(|s| s.series_uid == *series_uid.unwrap());
        if series.is_none() {
            return vec![];
        }

        let series = series.unwrap();

        // Check for specific instance
        if let Some(instance_uid) = params.get("00080018") {
            if !instance_uid.is_empty() {
                // Looking for specific instance
                for instance in &series.instances {
                    if instance.instance_uid == *instance_uid {
                        return vec![self.create_instance_response(series, instance)];
                    }
                }
                return vec![]; // Specific instance not found
            }
        }

        // Check instance number filter
        if let Some(instance_num_str) = params.get("00200013") {
            if !instance_num_str.is_empty() {
                if let Ok(instance_num) = instance_num_str.parse::<i32>() {
                    // Filter by instance number
                    return series
                        .instances
                        .iter()
                        .filter(|i| i.instance_number == instance_num)
                        .map(|i| self.create_instance_response(series, i))
                        .collect();
                }
            }
        }

        // Return all instances in the series
        series
            .instances
            .iter()
            .map(|i| self.create_instance_response(series, i))
            .collect()
    }

    fn create_series_response(&self, series: &MockSeries) -> serde_json::Value {
        serde_json::json!({
            "0020000D": {
                "vr": "UI",
                "Value": [self.study_uid]
            },
            "0020000E": {
                "vr": "UI",
                "Value": [series.series_uid]
            },
            "00200011": {
                "vr": "IS",
                "Value": [series.series_number.to_string()]
            },
            "00080060": {
                "vr": "CS",
                "Value": [series.modality]
            },
            "0008103E": {
                "vr": "LO",
                "Value": [series.series_description]
            },
            "00201209": {
                "vr": "IS",
                "Value": [series.instances.len().to_string()]
            }
        })
    }

    fn create_instance_response(
        &self,
        series: &MockSeries,
        instance: &MockInstance,
    ) -> serde_json::Value {
        serde_json::json!({
            "0020000D": {
                "vr": "UI",
                "Value": [self.study_uid]
            },
            "0020000E": {
                "vr": "UI",
                "Value": [series.series_uid]
            },
            "00080018": {
                "vr": "UI",
                "Value": [instance.instance_uid]
            },
            "00200013": {
                "vr": "IS",
                "Value": [instance.instance_number.to_string()]
            },
            "00080016": {
                "vr": "UI",
                "Value": [instance.sop_class_uid]
            },
            "00200011": {
                "vr": "IS",
                "Value": [series.series_number.to_string()]
            }
        })
    }
}

#[async_trait]
impl ServiceType for MockDicomEndpoint {
    fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Mock DICOM endpoint always validates successfully
        Ok(())
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Mock DICOM backend - no HTTP routes needed
        vec![]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        use crate::models::envelope::envelope::{RequestDetails, RequestEnvelope};
        use std::collections::HashMap as Map;

        if ctx.protocol != crate::models::protocol::Protocol::Dimse {
            return Err(Error::from(
                "MockDicomEndpoint only supports Protocol::Dimse in build_protocol_envelope",
            ));
        }

        // Build minimal RequestDetails using meta
        let metadata: Map<String, String> = ctx.meta.clone();
        let op = metadata
            .get("operation")
            .cloned()
            .unwrap_or_else(|| "DIMSE".into());
        let uri = format!("mock-dicom://scp/{}", op.to_lowercase());
        let details = RequestDetails {
            method: op,
            uri,
            headers: Map::new(),
            cookies: Map::new(),
            query_params: Map::new(),
            cache_status: None,
            metadata,
        };

        // Prefer normalized_data as the JSON body if payload is JSON
        let normalized: Option<serde_json::Value> = serde_json::from_slice(&ctx.payload).ok();

        Ok(RequestEnvelope {
            request_details: details,
            original_data: ctx.payload,
            normalized_data: normalized,
            normalized_snapshot: None,
        })
    }
}

#[async_trait]
impl ServiceHandler<Value> for MockDicomEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        self.handle_backend_request(&mut envelope, options).await
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

        if let Some(body_str) = response_meta
            .and_then(|m| m.get("body"))
            .and_then(|b| b.as_str())
        {
            return builder
                .body(Body::from(body_str.to_string()))
                .map_err(|_| Error::from("Failed to construct mock DICOM HTTP response"));
        }

        let body_str = serde_json::to_string(&nd).map_err(|_| {
            Error::from("Failed to serialize mock DICOM response payload into JSON")
        })?;

        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }

        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct mock DICOM HTTP response"))
    }
}

impl MockDicomEndpoint {
    /// Handle C-FIND operations
    async fn handle_c_find(
        &self,
        envelope: &RequestEnvelope<Vec<u8>>,
        path: &str,
    ) -> serde_json::Value {
        // Parse request body as either wrapper or raw identifier JSON
        let body_json: serde_json::Value =
            serde_json::from_slice(&envelope.original_data).unwrap_or(serde_json::Value::Null);

        // Extract identifier JSON
        let mut identifier_json = match body_json {
            serde_json::Value::Object(_) => {
                use dicom_json_tool as djt;
                let (_cmd, ident, _qmeta) = djt::parse_wrapper_or_identifier(&body_json);
                ident
            }
            _ => serde_json::json!({}),
        };

        if let Some(nd) = envelope.normalized_data.as_ref() {
            if let Some(ident) = nd.get("dimse_identifier") {
                if ident.is_object() {
                    identifier_json = ident.clone();
                }
            }
        }

        // Flatten identifier JSON into tag->string map for query parameters
        let mut params: HashMap<String, String> = HashMap::new();
        if let Some(map) = identifier_json.as_object() {
            for (tag, entry) in map.iter() {
                // Expect { vr: ..., Value: [...] }
                if let Some(val_array) = entry.get("Value").and_then(|v| v.as_array()) {
                    if let Some(first) = val_array.first() {
                        if let Some(s) = first.as_str() {
                            params.insert(tag.clone(), s.to_string());
                        } else if let Some(obj) = first.as_object() {
                            // PN case: { Alphabetic: "..." }
                            if let Some(alpha) = obj.get("Alphabetic").and_then(|v| v.as_str()) {
                                params.insert(tag.clone(), alpha.to_string());
                            }
                        }
                    } else {
                        // Empty array indicates return key
                        params.insert(tag.clone(), String::new());
                    }
                }
            }
        }

        // Debug logging for series and instances queries
        if path.contains("/series") || path.contains("/instances") {
            debug!("[MOCK DICOM] C-FIND Query:");
            debug!("[MOCK DICOM]   Path: {}", path);
            debug!("[MOCK DICOM]   Query: {:?}", params);
        }

        // Handle query using mock data
        let matches = MockDicomData::instance().handle_find_query(&params);

        if path.contains("/series") || path.contains("/instances") {
            debug!("[MOCK DICOM] Results:");
            debug!("[MOCK DICOM]   Matches found: {}", matches.len());
        }

        serde_json::json!({
            "operation": "find",
            "success": true,
            "matches": matches
        })
    }

    /// Handle C-GET operations for WADO-RS instance/frame retrieval
    async fn handle_c_get(
        &self,
        _envelope: &RequestEnvelope<Vec<u8>>,
        path: &str,
    ) -> serde_json::Value {
        debug!("[MOCK DICOM] C-GET Operation - Path: {}", path);

        // Create mock DICOM data directory and file
        let mock_dir = std::path::Path::new("/tmp/mock_dicom_data");
        if !mock_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(mock_dir) {
                debug!("[MOCK DICOM] Failed to create mock dir: {}", e);
            } else {
                // Create a minimal mock DICOM file
                let mock_file = mock_dir.join("instance1.dcm");
                if let Err(e) = std::fs::write(&mock_file, b"MOCK DICOM FILE DATA") {
                    debug!("[MOCK DICOM] Failed to create mock DICOM file: {}", e);
                }
            }
        }

        // Check if this is a frame request
        if path.contains("/frames/") {
            // Handle frame retrieval - return error for out-of-range frames
            let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            if let Some(frame_part) = parts.last() {
                if let Ok(frame_num) = frame_part.parse::<u32>() {
                    if frame_num >= 9999 {
                        // Out of range frame - mock error
                        return serde_json::json!({
                            "operation": "get",
                            "success": false,
                            "error": "Frame number out of range"
                        });
                    } else if frame_num >= 1 {
                        // Normal frame request - mock success
                        return serde_json::json!({
                            "operation": "get",
                            "success": true,
                            "folder_path": "/tmp/mock_dicom_data"
                        });
                    }
                }
            }
        }

        // For regular instance retrieval, return mock success with folder path
        serde_json::json!({
            "operation": "get",
            "success": true,
            "folder_path": "/tmp/mock_dicom_data",
            "instances": [
                {
                    "sop_instance_uid": MockDicomData::instance().series[0].instances[0].instance_uid,
                    "file_path": "/tmp/mock_dicom_data/instance1.dcm"
                }
            ]
        })
    }

    /// Handle backend (SCU) request processing for mock DICOM
    async fn handle_backend_request(
        &self,
        envelope: &mut RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Extract path for context and resolve operation (check normalized_data first)
        let path = envelope
            .normalized_data
            .as_ref()
            .and_then(|nd| nd.get("path").and_then(|p| p.as_str()))
            .map(|s| s.to_string())
            .or_else(|| envelope.request_details.metadata.get("path").cloned())
            .unwrap_or_default();
            
        let op = envelope
            .normalized_data
            .as_ref()
            .and_then(|nd| nd.get("dimse_op").and_then(|op| op.as_str()))
            .map(|s| s.to_string())
            .or_else(|| envelope.request_details.metadata.get("dimse_op").cloned())
            .unwrap_or_else(|| path.clone());

        let result = match op.as_str() {
            "echo" | "/echo" => {
                // Mock C-ECHO - always successful
                serde_json::json!({
                    "operation": "echo",
                    "success": true,
                    "remote_aet": "MOCK_DICOM",
                    "host": "mock",
                    "port": 11112
                })
            }
            "find" | "/find" => self.handle_c_find(envelope, &path).await,
            "get" | "/get" => self.handle_c_get(envelope, &path).await,
            _ => {
                serde_json::json!({
                    "operation": op,
                    "success": false,
                    "error": format!("Mock DICOM: Unsupported operation: {}", op)
                })
            }
        };

        // Update envelope with mock result
        envelope.normalized_data = Some(result);
        Ok(envelope.clone())
    }
}
