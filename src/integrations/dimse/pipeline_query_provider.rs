use crate::globals::get_config;
use crate::models::envelope::envelope::ResponseEnvelope;
use crate::pipeline::executor::PipelineExecutor;
use async_trait::async_trait;
use bytes::Bytes;
use dicom_json_tool as tool;
use dimse::error::DimseError;
use dimse::types::{DatasetMetadata, DatasetStream, QueryLevel};
use dimse::Result as DimseResult;
use std::collections::HashMap;
use tracing::{debug, warn};

use once_cell::sync::Lazy;
use std::path::PathBuf;
use std::sync::Mutex;

static CURRENT_STORE_DIR: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

pub fn set_current_store_dir<P: Into<PathBuf>>(dir: P) {
    let mut guard = CURRENT_STORE_DIR.lock().expect("store dir mutex");
    *guard = Some(dir.into());
}

fn get_current_store_dir() -> Option<PathBuf> {
    CURRENT_STORE_DIR.lock().ok().and_then(|g| g.clone())
}

pub struct PipelineQueryProvider {
    pipeline: String,
    endpoint: String,
}

impl PipelineQueryProvider {
    pub fn new(pipeline: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            pipeline: pipeline.into(),
            endpoint: endpoint.into(),
        }
    }

    fn build_identifier_json(&self, parameters: &HashMap<String, String>) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (tag, val) in parameters.iter() {
            let vr = match tag.as_str() {
                "00100010" => "PN",
                "00100020" => "LO",
                "00080020" => "DA",
                _ => "UN",
            };
            let value = if val.is_empty() {
                serde_json::Value::Array(vec![])
            } else if vr == "PN" {
                serde_json::json!([{ "Alphabetic": val }])
            } else {
                serde_json::json!([val])
            };
            map.insert(tag.clone(), serde_json::json!({ "vr": vr, "Value": value }));
        }
        serde_json::Value::Object(map)
    }

    fn build_query_metadata(
        &self,
        parameters: &HashMap<String, String>,
    ) -> tool::model::QueryMetadata {
        let mut out: HashMap<String, tool::model::QueryMetaEntry> = HashMap::new();
        for (tag, val) in parameters.iter() {
            let match_type = if val.is_empty() {
                "RETURN_KEY"
            } else if val.contains('*') || val.contains('?') {
                "WILDCARD"
            } else if (tag == "00080020" || tag == "00080021") && val.contains('-') {
                "RANGE"
            } else {
                "EXACT"
            };
            out.insert(
                tag.clone(),
                tool::model::QueryMetaEntry {
                    match_type: Some(match_type.into()),
                },
            );
        }
        tool::model::QueryMetadata(out)
    }

    async fn run(
        &self,
        op: &str,
        body: serde_json::Value,
        mut meta: HashMap<String, String>,
    ) -> DimseResult<ResponseEnvelope<Vec<u8>>> {
        use crate::models::protocol::{Protocol, ProtocolCtx};
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;

        // Resolve endpoint service and options
        let endpoint = config.endpoints.get(&self.endpoint).ok_or_else(|| {
            DimseError::operation_failed(format!("Unknown endpoint '{}'", self.endpoint))
        })?;
        let service = endpoint
            .resolve_service()
            .map_err(|e| DimseError::operation_failed(format!("Resolve service failed: {}", e)))?;
        let options_owned: HashMap<String, serde_json::Value> =
            endpoint.options.clone().unwrap_or_default();
        let options = &options_owned;

        // ProtocolCtx for DIMSE
        meta.insert("protocol".into(), "dimse".into());
        meta.insert("operation".into(), op.to_string());
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&body).unwrap_or_default(),
            meta,
            attrs: serde_json::json!({}),
        };

        // Let the service build the envelope
        let envelope = service
            .build_protocol_envelope(ctx.clone(), options)
            .await
            .map_err(|e| DimseError::operation_failed(format!("Envelope build failed: {}", e)))?;

        // Get pipeline reference
        let pipeline = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Unknown pipeline '{}'", self.pipeline))
        })?;

        // Execute through PipelineExecutor (single source of truth)
        let response = PipelineExecutor::execute(envelope, pipeline, &config, &ctx)
            .await
            .map_err(|e| DimseError::operation_failed(format!("Pipeline failed: {}", e)))?;

        Ok(response)
    }

    /// Extract C-FIND results from QIDO-RS JSON response
    fn extract_cfind_results(
        &self,
        response: &ResponseEnvelope<Vec<u8>>,
    ) -> DimseResult<Vec<DatasetStream>> {
        let http_status = response.response_details.status;
        let dicom_status = response
            .response_details
            .metadata
            .get("dicom_status")
            .map(|s| s.as_str())
            .unwrap_or("0x0000");

        debug!(
            "Extracting C-FIND results: http_status={}, dicom_status={}, payload_size={}",
            http_status,
            dicom_status,
            response.original_data.len()
        );

        // Check for failure status
        if dicom_status.starts_with("0xC") || dicom_status.starts_with("0xA") {
            return Err(DimseError::operation_failed(format!(
                "C-FIND failed with DICOM status {}",
                dicom_status
            )));
        }

        // Get results from normalized_data (set by dicom_to_dicomweb middleware)
        let results = if let Some(ref data) = response.normalized_data {
            if let Some(arr) = data.as_array() {
                arr.clone()
            } else {
                vec![]
            }
        } else if !response.original_data.is_empty() {
            // Try to parse from raw response
            match serde_json::from_slice::<serde_json::Value>(&response.original_data) {
                Ok(serde_json::Value::Array(arr)) => arr,
                Ok(obj) => vec![obj],
                Err(e) => {
                    warn!("Failed to parse C-FIND response as JSON: {}", e);
                    vec![]
                }
            }
        } else {
            vec![]
        };

        debug!("C-FIND extracted {} results", results.len());

        // Convert each JSON object to a DatasetStream
        let datasets: Vec<DatasetStream> = results
            .into_iter()
            .map(|json_obj| {
                // Store the JSON as bytes in a DatasetStream::Memory
                let json_bytes = serde_json::to_vec(&json_obj).unwrap_or_default();
                let mut metadata = DatasetMetadata::new();

                // Extract UIDs from the JSON for metadata
                if let Some(study_uid) = json_obj
                    .get("0020000D")
                    .and_then(|v| v.get("Value"))
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.as_str())
                {
                    metadata.study_instance_uid = Some(study_uid.to_string());
                }
                if let Some(series_uid) = json_obj
                    .get("0020000E")
                    .and_then(|v| v.get("Value"))
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.as_str())
                {
                    metadata.series_instance_uid = Some(series_uid.to_string());
                }
                if let Some(sop_uid) = json_obj
                    .get("00080018")
                    .and_then(|v| v.get("Value"))
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.as_str())
                {
                    metadata.sop_instance_uid = Some(sop_uid.to_string());
                }
                if let Some(patient_id) = json_obj
                    .get("00100020")
                    .and_then(|v| v.get("Value"))
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.as_str())
                {
                    metadata.patient_id = Some(patient_id.to_string());
                }

                metadata.size_bytes = Some(json_bytes.len() as u64);

                DatasetStream::Memory {
                    data: Bytes::from(json_bytes),
                    metadata,
                }
            })
            .collect();

        Ok(datasets)
    }

    /// Extract datasets from WADO-RS response (for C-GET/C-MOVE)
    fn extract_wado_results(
        &self,
        response: &ResponseEnvelope<Vec<u8>>,
        operation: &str,
    ) -> DimseResult<Vec<DatasetStream>> {
        let http_status = response.response_details.status;
        let dicom_status = response
            .response_details
            .metadata
            .get("dicom_status")
            .map(|s| s.as_str())
            .unwrap_or("0x0000");
        let response_format = response
            .response_details
            .metadata
            .get("response_format")
            .map(|s| s.as_str())
            .unwrap_or("");

        debug!(
            "Extracting {} results: http_status={}, dicom_status={}, format={}, payload_size={}",
            operation,
            http_status,
            dicom_status,
            response_format,
            response.original_data.len()
        );

        // Check for failure status
        if dicom_status.starts_with("0xC") || dicom_status.starts_with("0xA") {
            return Err(DimseError::operation_failed(format!(
                "{} failed with DICOM status {}",
                operation, dicom_status
            )));
        }

        // Handle different response formats
        if response_format == "multipart/dicom" {
            // Multipart response - need to parse the parts
            let boundary = response
                .response_details
                .metadata
                .get("multipart_boundary")
                .map(|s| s.as_str())
                .unwrap_or("");

            if boundary.is_empty() {
                warn!("{} multipart response missing boundary", operation);
                return Ok(vec![]);
            }

            // Parse multipart body
            self.parse_multipart_body(&response.original_data, boundary)
        } else if !response.original_data.is_empty() {
            // Single DICOM instance or raw bytes
            debug!(
                "{} received single dataset of {} bytes",
                operation,
                response.original_data.len()
            );

            let metadata = DatasetMetadata::new();
            Ok(vec![DatasetStream::Memory {
                data: Bytes::from(response.original_data.clone()),
                metadata,
            }])
        } else {
            debug!("{} response has no data", operation);
            Ok(vec![])
        }
    }

    /// Parse multipart/related body and extract DICOM parts
    fn parse_multipart_body(
        &self,
        body: &[u8],
        boundary: &str,
    ) -> DimseResult<Vec<DatasetStream>> {
        let mut datasets = Vec::new();
        let boundary_bytes = format!("--{}", boundary);
        let terminator = format!("--{}--", boundary);

        // Convert body to string for easier parsing
        // Note: This is a simplified parser; a production implementation
        // should handle binary boundaries properly
        let body_str = String::from_utf8_lossy(body);

        // Split by boundary
        let parts: Vec<&str> = body_str.split(&boundary_bytes).collect();

        for part in parts.iter().skip(1) {
            // Skip first empty part
            // Skip terminator
            if part.starts_with("--") || part.trim().is_empty() {
                continue;
            }

            // Find the blank line separating headers from content
            if let Some(header_end) = part.find("\r\n\r\n") {
                let headers = &part[..header_end];
                let content = &part[header_end + 4..];

                // Check if this part is DICOM
                if headers.to_lowercase().contains("application/dicom") {
                    // Remove trailing boundary markers
                    let content = content.trim_end_matches(&terminator).trim();
                    let content_bytes = content.as_bytes().to_vec();

                    if !content_bytes.is_empty() {
                        debug!("Extracted DICOM part of {} bytes", content_bytes.len());
                        let metadata = DatasetMetadata::new();
                        datasets.push(DatasetStream::Memory {
                            data: Bytes::from(content_bytes),
                            metadata,
                        });
                    }
                }
            }
        }

        debug!("Parsed {} DICOM parts from multipart response", datasets.len());
        Ok(datasets)
    }
}

#[async_trait]
impl dimse::scp::QueryProvider for PipelineQueryProvider {
    async fn find(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
        max_results: u32,
    ) -> DimseResult<Vec<DatasetStream>> {
        let mut meta = HashMap::new();
        meta.insert("dicom.operation".into(), "C-FIND".into());
        meta.insert("dicom.query_level".into(), format!("{}", query_level));
        meta.insert("dicom.max_results".into(), max_results.to_string());

        // Build wrapper for pipeline
        let cmd = tool::model::CommandMeta {
            message_id: Some(1),
            sop_class_uid: None,
            priority: Some("MEDIUM".into()),
            direction: Some("REQUEST".into()),
        };
        let identifier = self.build_identifier_json(parameters);
        let qmeta = self.build_query_metadata(parameters);
        let wrapper = tool::model::Wrapper {
            command: Some(cmd),
            identifier,
            query_metadata: Some(qmeta),
        };
        let body = serde_json::to_value(&wrapper)
            .map_err(|e| DimseError::operation_failed(format!("Wrapper serialize: {}", e)))?;

        let response_envelope = self.run("C-FIND", body, meta).await?;

        // Extract C-FIND results from the middleware-transformed response
        self.extract_cfind_results(&response_envelope)
    }

    async fn locate(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<Vec<DatasetStream>> {
        let mut meta = HashMap::new();
        meta.insert("dicom.operation".into(), "C-MOVE".into());
        meta.insert("dicom.query_level".into(), format!("{}", query_level));

        let cmd = tool::model::CommandMeta {
            message_id: Some(1),
            sop_class_uid: None,
            priority: Some("MEDIUM".into()),
            direction: Some("REQUEST".into()),
        };
        let identifier = self.build_identifier_json(parameters);
        let qmeta = self.build_query_metadata(parameters);
        let wrapper = tool::model::Wrapper {
            command: Some(cmd),
            identifier,
            query_metadata: Some(qmeta),
        };
        let body = serde_json::to_value(&wrapper)
            .map_err(|e| DimseError::operation_failed(format!("Wrapper serialize: {}", e)))?;

        let response_envelope = self.run("C-MOVE", body, meta).await?;

        // Extract datasets from the middleware-transformed response
        self.extract_wado_results(&response_envelope, "C-MOVE")
    }

    async fn get(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<Vec<DatasetStream>> {
        let mut meta = HashMap::new();
        meta.insert("dicom.operation".into(), "C-GET".into());
        meta.insert("dicom.query_level".into(), format!("{}", query_level));

        let cmd = tool::model::CommandMeta {
            message_id: Some(1),
            sop_class_uid: None,
            priority: Some("MEDIUM".into()),
            direction: Some("REQUEST".into()),
        };
        let identifier = self.build_identifier_json(parameters);
        let qmeta = self.build_query_metadata(parameters);
        let wrapper = tool::model::Wrapper {
            command: Some(cmd),
            identifier,
            query_metadata: Some(qmeta),
        };
        let body = serde_json::to_value(&wrapper)
            .map_err(|e| DimseError::operation_failed(format!("Wrapper serialize: {}", e)))?;

        let response_envelope = self.run("C-GET", body, meta).await?;

        // Extract datasets from the middleware-transformed response
        self.extract_wado_results(&response_envelope, "C-GET")
    }

    async fn store(&self, dataset: DatasetStream) -> DimseResult<()> {
        // Write incoming dataset into the current per-move directory if set, otherwise default
        let target_dir = get_current_store_dir().unwrap_or_else(|| PathBuf::from("./tmp/dimse"));
        if let Err(e) = tokio::fs::create_dir_all(&target_dir).await {
            return Err(DimseError::operation_failed(format!(
                "ensure store dir: {}",
                e
            )));
        }
        let _temp = dataset
            .to_temp_file(&target_dir)
            .await
            .map_err(|e| DimseError::operation_failed(format!("store dataset: {}", e)))?;

        // Emit pipeline event for observability and processing
        let mut meta = HashMap::new();
        meta.insert("dicom.operation".into(), "C-STORE".into());
        let body = serde_json::json!({
            "operation": "store",
            "dir": target_dir.to_string_lossy(),
        });

        // Check response from middleware for DICOM status
        match self.run("C-STORE", body, meta).await {
            Ok(response) => {
                let http_status = response.response_details.status;
                let dicom_status = response
                    .response_details
                    .metadata
                    .get("dicom_status")
                    .map(|s| s.as_str())
                    .unwrap_or("0x0000");

                debug!(
                    "C-STORE pipeline response: http_status={}, dicom_status={}",
                    http_status, dicom_status
                );

                // Check if DICOM status indicates failure
                if dicom_status.starts_with("0xC") || dicom_status.starts_with("0xA") {
                    return Err(DimseError::operation_failed(format!(
                        "C-STORE failed with DICOM status {}",
                        dicom_status
                    )));
                }

                Ok(())
            }
            Err(e) => {
                tracing::error!("C-STORE pipeline failed: {}", e);
                Err(e)
            }
        }
    }
}
