use crate::adapters::dimse::status_mapper;
use crate::globals::get_config;
use crate::models::envelope::envelope::ResponseEnvelope;
use crate::models::protocol::{Protocol, ProtocolCtx};
use crate::pipeline::executor::PipelineExecutor;
use async_trait::async_trait;
use dicom_json_tool as tool;
use dimse::error::DimseError;
use dimse::types::{DatasetStream, QueryLevel};
use dimse::Result as DimseResult;
use once_cell::sync::Lazy;
use std::collections::HashMap;
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

/// DIMSE query provider that integrates with the pipeline system
/// 
/// This provider implements the dimse::scp::QueryProvider trait to handle
/// C-FIND, C-MOVE, and C-STORE operations by converting them to protocol
/// contexts and executing them through the PipelineExecutor.
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

    /// Build DICOM JSON identifier from DIMSE query parameters
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

    /// Build query metadata with match types
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

    /// Execute a DIMSE operation through the pipeline
    /// 
    /// Converts DIMSE operation to protocol context, builds envelope via service,
    /// executes pipeline, and returns response envelope.
    async fn run(
        &self,
        op: &str,
        body: serde_json::Value,
        mut meta: HashMap<String, String>,
    ) -> DimseResult<ResponseEnvelope<Vec<u8>>> {
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

        // Build ProtocolCtx for DIMSE
        meta.insert("protocol".into(), "dimse".into());
        meta.insert("operation".into(), op.to_string());
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&body).unwrap_or_default(),
            meta,
            attrs: serde_json::json!({}),
        };

        // Let the service build the request envelope
        let request_envelope = service
            .build_protocol_envelope(ctx.clone(), options)
            .await
            .map_err(|e| DimseError::operation_failed(format!("Envelope build failed: {}", e)))?;

        // Get pipeline configuration
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        // Execute pipeline using PipelineExecutor
        let response_envelope = PipelineExecutor::execute(
            request_envelope,
            pipeline_cfg,
            &config,
            &ctx,
        )
        .await
        .map_err(|e| DimseError::operation_failed(format!("Pipeline execution failed: {}", e)))?;

        Ok(response_envelope)
    }

    /// Convert ResponseEnvelope to Vec<DatasetStream>
    ///
    /// Parses the response data and converts it to DICOM datasets.
    /// Supports both single results and arrays of results.
    async fn response_to_datasets(
        &self,
        response: ResponseEnvelope<Vec<u8>>,
    ) -> DimseResult<Vec<DatasetStream>> {
        // Check HTTP status and map errors
        let http_status = response.response_details.status;
        if !(200..300).contains(&http_status) {
            let status = status_mapper::http_status_to_dimse(http_status);
            return Err(DimseError::operation_failed(format!(
                "Pipeline returned non-success status {}: {:?}",
                http_status, status
            )));
        }

        // Try to parse response body as JSON
        let json_data = if let Some(normalized) = response.normalized_data {
            // Use normalized_data if available
            normalized
        } else if !response.original_data.is_empty() {
            // Try to parse original_data as JSON
            serde_json::from_slice(&response.original_data)
                .map_err(|e| DimseError::operation_failed(format!("Failed to parse response as JSON: {}", e)))?
        } else {
            // Empty response
            return Ok(vec![]);
        };

        // Convert JSON to datasets
        let datasets = match &json_data {
            serde_json::Value::Array(items) => {
                // Multiple results
                let mut datasets = Vec::new();
                for item in items {
                    if let Some(dataset) = self.json_to_dataset(item).await? {
                        datasets.push(dataset);
                    }
                }
                datasets
            }
            serde_json::Value::Object(_) => {
                // Single result
                if let Some(dataset) = self.json_to_dataset(&json_data).await? {
                    vec![dataset]
                } else {
                    vec![]
                }
            }
            _ => {
                // Unexpected format
                return Err(DimseError::operation_failed(format!(
                    "Unexpected response format: expected object or array, got {:?}",
                    json_data
                )));
            }
        };

        Ok(datasets)
    }

    /// Convert a single JSON object to a DatasetStream
    ///
    /// Currently creates in-memory datasets from JSON.
    /// Future: could support file-based datasets for large results.
    async fn json_to_dataset(
        &self,
        json: &serde_json::Value,
    ) -> DimseResult<Option<DatasetStream>> {
        // Skip null or empty objects
        if json.is_null() || (json.is_object() && json.as_object().unwrap().is_empty()) {
            return Ok(None);
        }

        // Convert JSON back to bytes for DatasetStream
        // In the future, we could parse DICOM JSON and build proper DICOM objects
        let json_bytes = serde_json::to_vec(json)
            .map_err(|e| DimseError::operation_failed(format!("Failed to serialize JSON: {}", e)))?;

        // DatasetStream::from_bytes expects bytes::Bytes, but it's just a wrapper around Vec<u8>
        // We convert via into() which should work since Bytes implements From<Vec<u8>>
        let dataset = DatasetStream::from_bytes(json_bytes.into());
        Ok(Some(dataset))
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
        
        // Convert response envelope to datasets
        self.response_to_datasets(response_envelope).await
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
        
        // Convert response envelope to datasets
        self.response_to_datasets(response_envelope).await
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

        // Also emit a pipeline event for observability (optional)
        let mut meta = HashMap::new();
        meta.insert("dicom.operation".into(), "C-STORE".into());
        let body = serde_json::json!({
            "operation": "store",
            "dir": target_dir.to_string_lossy(),
        });
        let _ = self.run("C-STORE", body, meta).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_provider_creation() {
        let provider = PipelineQueryProvider::new("test_pipeline", "test_endpoint");
        assert_eq!(provider.pipeline, "test_pipeline");
        assert_eq!(provider.endpoint, "test_endpoint");
    }

    #[test]
    fn test_build_identifier_json_empty() {
        let provider = PipelineQueryProvider::new("p", "e");
        let params = HashMap::new();
        let json = provider.build_identifier_json(&params);
        assert!(json.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_build_identifier_json_patient_name() {
        let provider = PipelineQueryProvider::new("p", "e");
        let mut params = HashMap::new();
        params.insert("00100010".to_string(), "Doe^John".to_string());
        
        let json = provider.build_identifier_json(&params);
        let obj = json.as_object().unwrap();
        
        assert!(obj.contains_key("00100010"));
        let entry = &obj["00100010"];
        assert_eq!(entry["vr"], "PN");
        assert_eq!(entry["Value"][0]["Alphabetic"], "Doe^John");
    }

    #[test]
    fn test_build_query_metadata_exact_match() {
        let provider = PipelineQueryProvider::new("p", "e");
        let mut params = HashMap::new();
        params.insert("00100020".to_string(), "12345".to_string());
        
        let meta = provider.build_query_metadata(&params);
        assert_eq!(
            meta.0.get("00100020").unwrap().match_type.as_deref(),
            Some("EXACT")
        );
    }

    #[test]
    fn test_build_query_metadata_wildcard() {
        let provider = PipelineQueryProvider::new("p", "e");
        let mut params = HashMap::new();
        params.insert("00100010".to_string(), "Doe*".to_string());
        
        let meta = provider.build_query_metadata(&params);
        assert_eq!(
            meta.0.get("00100010").unwrap().match_type.as_deref(),
            Some("WILDCARD")
        );
    }

    #[test]
    fn test_build_query_metadata_return_key() {
        let provider = PipelineQueryProvider::new("p", "e");
        let mut params = HashMap::new();
        params.insert("00080020".to_string(), "".to_string());
        
        let meta = provider.build_query_metadata(&params);
        assert_eq!(
            meta.0.get("00080020").unwrap().match_type.as_deref(),
            Some("RETURN_KEY")
        );
    }

    #[test]
    fn test_store_dir_functions() {
        // Test get/set current store dir
        set_current_store_dir(PathBuf::from("/test/path"));
        let retrieved = get_current_store_dir();
        assert_eq!(retrieved, Some(PathBuf::from("/test/path")));
    }
}
