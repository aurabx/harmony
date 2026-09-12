use crate::adapters::dimse::envelope_unwrapper::{unwrap_response, UnwrappedContent};
use crate::adapters::dimse::status_mapper;
use crate::globals::{get_config, get_storage};
use crate::models::envelope::command::{Command, Operation};
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
use std::sync::{Arc, Mutex};

static CURRENT_STORE_DIR: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

// pub fn set_current_store_dir<P: Into<PathBuf>>(dir: P) {
//     let mut guard = CURRENT_STORE_DIR.lock().expect("store dir mutex");
//     *guard = Some(dir.into());
// }

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
        command: Command,
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

        // Build ProtocolCtx for DIMSE with typed Command.
        // The Command is the single source of truth for the operation.
        meta.insert("protocol".into(), "dimse".into());
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&body).unwrap_or_default(),
            meta,
            attrs: serde_json::json!({}),
            command: Some(command),
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
        let response_envelope =
            PipelineExecutor::execute(request_envelope, &self.pipeline, pipeline_cfg, &config, &ctx)
                .await
                .map_err(|e| {
                    DimseError::operation_failed(format!("Pipeline execution failed: {}", e))
                })?;

        Ok(response_envelope)
    }

    /// Execute a DIMSE operation through the pipeline with streaming support
    ///
    /// Similar to `run()` but uses `execute_streaming()` for backends that support it.
    /// Returns a stream of datasets that can be processed progressively.
    async fn run_streaming(
        &self,
        op: &str,
        body: serde_json::Value,
        mut meta: HashMap<String, String>,
    ) -> DimseResult<Vec<DatasetStream>> {
        use futures_util::StreamExt;

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
        meta.insert("dimse_op".into(), op.to_string());
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&body).unwrap_or_default(),
            meta,
            attrs: serde_json::json!({}),
            command: None,
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

        tracing::info!("Executing streaming pipeline for DIMSE operation: {}", op);

        // Execute streaming pipeline
        let mut response_stream =
            PipelineExecutor::execute_streaming(request_envelope, &self.pipeline, pipeline_cfg, &config, &ctx)
                .await
                .map_err(|e| {
                    DimseError::operation_failed(format!("Streaming pipeline execution failed: {}", e))
                })?;

        // Collect datasets from stream
        let mut all_datasets = Vec::new();
        let mut instance_count = 0u64;

        while let Some(result) = response_stream.next().await {
            match result {
                Ok(response_envelope) => {
                    instance_count += 1;
                    tracing::debug!("Processing streamed instance #{}", instance_count);

                    // Convert this envelope to datasets
                    let datasets = self.response_to_datasets(response_envelope).await?;
                    all_datasets.extend(datasets);
                }
                Err(e) => {
                    tracing::error!("Stream error on instance #{}: {}", instance_count, e);
                    return Err(DimseError::operation_failed(format!(
                        "Stream failed after {} instances: {}",
                        instance_count, e
                    )));
                }
            }
        }

        tracing::info!(
            "Streaming pipeline completed: {} instances, {} total datasets",
            instance_count,
            all_datasets.len()
        );

        Ok(all_datasets)
    }

    /// Convert ResponseEnvelope to Vec<DatasetStream>
    ///
    /// Parses the response data and converts it to DICOM datasets.
    /// Uses the envelope_unwrapper module to handle various response formats.
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
            normalized
        } else if !response.original_data.is_empty() {
            serde_json::from_slice(&response.original_data).map_err(|e| {
                DimseError::operation_failed(format!("Failed to parse response as JSON: {}", e))
            })?
        } else {
            return Ok(vec![]);
        };

        // Use envelope unwrapper to extract clean DICOM content
        match unwrap_response(&json_data) {
            UnwrappedContent::DicomJson(items) => {
                tracing::debug!("response_to_datasets: unwrapped {} DICOM JSON objects", items.len());
                let mut datasets = Vec::new();
                for item in items {
                    match self.dicom_json_to_dataset(&item) {
                        Ok(dataset) => datasets.push(dataset),
                        Err(e) => {
                            tracing::warn!("Failed to convert DICOM JSON to dataset: {}", e);
                        }
                    }
                }
                Ok(datasets)
            }
            UnwrappedContent::Binary(bytes) => {
                tracing::debug!("response_to_datasets: unwrapped binary DICOM data ({} bytes)", bytes.len());
                let dataset = self.binary_to_dataset(&bytes)?;
                Ok(vec![dataset])
            }
            UnwrappedContent::FolderPath(folder_path) => {
                tracing::debug!("response_to_datasets: reading DICOM files from folder: {:?}", folder_path);
                self.folder_to_datasets(&folder_path)
            }
            UnwrappedContent::Empty => {
                tracing::debug!("response_to_datasets: no content to convert");
                Ok(vec![])
            }
        }
    }

    /// Convert clean DICOM JSON to a DatasetStream
    ///
    /// This function expects already-cleaned DICOM JSON (8-character hex tag keys).
    /// Use envelope_unwrapper to extract clean JSON before calling this.
    fn dicom_json_to_dataset(&self, json: &serde_json::Value) -> DimseResult<DatasetStream> {
        let dicom_obj = tool::json_value_to_identifier(json).map_err(|e| {
            tracing::error!(
                "dicom_json_to_dataset: conversion failed: {}. Keys: {:?}",
                e,
                json.as_object().map(|o| o.keys().collect::<Vec<_>>())
            );
            DimseError::operation_failed(format!("Failed to convert JSON to DICOM object: {}", e))
        })?;
        Ok(DatasetStream::from_object(dicom_obj))
    }

    /// Convert binary DICOM Part10 data to a DatasetStream
    fn binary_to_dataset(&self, bytes: &[u8]) -> DimseResult<DatasetStream> {
        let file_obj = dicom_object::from_reader(bytes).map_err(|e| {
            DimseError::operation_failed(format!("Failed to parse DICOM from binary: {}", e))
        })?;
        Ok(DatasetStream::from_object(file_obj.into_inner()))
    }

    /// Read all DICOM files from a folder and convert to DatasetStreams
    ///
    /// This is used for C-GET/C-MOVE results where files are stored on disk.
    fn folder_to_datasets(&self, folder_path: &std::path::Path) -> DimseResult<Vec<DatasetStream>> {
        let mut datasets = Vec::new();

        let entries = std::fs::read_dir(folder_path).map_err(|e| {
            DimseError::operation_failed(format!("Failed to read folder {:?}: {}", folder_path, e))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Check for .dcm extension or try to parse anyway
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if ext != "dcm" && ext != "dicom" {
                // Try to detect DICOM by reading magic bytes
                if let Ok(bytes) = std::fs::read(&path) {
                    if bytes.len() < 132 || &bytes[128..132] != b"DICM" {
                        tracing::debug!("Skipping non-DICOM file: {:?}", path);
                        continue;
                    }
                } else {
                    continue;
                }
            }

            tracing::debug!("Reading DICOM file: {:?}", path);

            // Create DatasetStream::File variant pointing to the file
            // This allows the SCP to send the file directly without re-parsing
            // Note: delete_on_drop is false since we don't own these files
            let dataset = DatasetStream::from_file(path.clone(), false);
            datasets.push(dataset);
        }

        tracing::info!("folder_to_datasets: loaded {} DICOM files from {:?}", datasets.len(), folder_path);
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
        meta.insert("query_level".into(), format!("{}", query_level));
        meta.insert("max_results".into(), max_results.to_string());

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

        // Check if pipeline supports streaming
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        if PipelineExecutor::is_streaming_pipeline(pipeline_cfg, &config) {
            tracing::info!("Using streaming pipeline for C-FIND");
            self.run_streaming("find", body, meta).await
        } else {
            let response_envelope = self
            .run(
                Command::new(Operation::Query).with_source("dimse_adapter"),
                body,
                meta,
            )
            .await?;
            self.response_to_datasets(response_envelope).await
        }
    }

    async fn locate(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<Vec<DatasetStream>> {
        let mut meta = HashMap::new();
        meta.insert("query_level".into(), format!("{}", query_level));

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

        let response_envelope = self
            .run(
                Command::new(Operation::Move).with_source("dimse_adapter"),
                body,
                meta,
            )
            .await?;

        // Convert response envelope to datasets
        self.response_to_datasets(response_envelope).await
    }

    async fn get(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<Vec<DatasetStream>> {
        let mut meta = HashMap::new();
        meta.insert("query_level".into(), format!("{}", query_level));

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

        // Check if pipeline supports streaming
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        if PipelineExecutor::is_streaming_pipeline(pipeline_cfg, &config) {
            tracing::info!("Using streaming pipeline for C-GET");
            self.run_streaming("get", body, meta).await
        } else {
            let response_envelope = self
                .run(
                    Command::new(Operation::Retrieve).with_source("dimse_adapter"),
                    body,
                    meta,
                )
                .await?;
            self.response_to_datasets(response_envelope).await
        }
    }

    async fn store(&self, dataset: DatasetStream) -> DimseResult<()> {
        // Resolve storage directory using global storage backend if available, otherwise fallback
        let target_dir = if let Some(storage) = get_storage() {
            // For filesystem backend, ensure_dir_str returns the full path
            storage
                .ensure_dir_str("dimse")
                .map_err(|e| DimseError::operation_failed(format!("Storage error: {}", e)))?
        } else {
            // Fallback for legacy behavior or when no storage backend configured
            let dir = get_current_store_dir().unwrap_or_else(|| PathBuf::from("./tmp/dimse"));
            if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                return Err(DimseError::operation_failed(format!(
                    "ensure store dir: {}",
                    e
                )));
            }
            dir
        };

        // Convert first instance to DICOM JSON for pipeline metadata
        // This allows middleware and storage backends to access DICOM tags
        let dicom_json = match dataset.to_object().await {
            Ok(obj) => match tool::identifier_to_json_value(&obj) {
                Ok(json) => Some(json),
                Err(e) => {
                    tracing::warn!("Failed to convert DICOM to JSON: {}", e);
                    None
                }
            },
            Err(e) => {
                tracing::warn!("Failed to parse DICOM object: {}", e);
                None
            }
        };

        let temp_file = dataset
            .to_temp_file(&target_dir)
            .await
            .map_err(|e| DimseError::operation_failed(format!("store dataset: {}", e)))?;

        // Build metadata from DICOM tags for path resolution
        let mut meta = HashMap::new();

        // Extract key UIDs from dataset metadata for path templating
        let ds_meta = dataset.metadata();
        if let Some(ref uid) = ds_meta.study_instance_uid {
            meta.insert("StudyInstanceUID".into(), uid.clone());
        }
        if let Some(ref uid) = ds_meta.series_instance_uid {
            meta.insert("SeriesInstanceUID".into(), uid.clone());
        }
        if let Some(ref uid) = ds_meta.sop_instance_uid {
            meta.insert("SOPInstanceUID".into(), uid.clone());
        }
        if let Some(ref id) = ds_meta.patient_id {
            meta.insert("PatientID".into(), id.clone());
        }

        // Build body with command metadata, dataset, and file path for STOW-RS transformation
        let body = serde_json::json!({
            "command": {
                "message_id": 1,
                "sop_class_uid": ds_meta.sop_class_uid,
                "priority": "MEDIUM",
                "direction": "REQUEST"
            },
            "dataset": dicom_json,
            "file": temp_file.to_string_lossy()
        });

        // Execute pipeline and check for DICOM status in response
        let response = self
            .run(
                Command::new(Operation::Store).with_source("dimse_adapter"),
                body,
                meta,
            )
            .await?;

        // Check for DICOM status in response metadata (set by dicom_to_dicomweb middleware)
        if let Some(status_str) = response.response_details.metadata.get("dicom_status") {
            // Parse "0x0000" or "0xC000" format
            if let Some(hex) = status_str.strip_prefix("0x") {
                if let Ok(status) = u16::from_str_radix(hex, 16) {
                    // Check for failure status (0xC000 range indicates failure)
                    if status >= 0xC000 {
                        return Err(DimseError::operation_failed(format!(
                            "C-STORE failed with DICOM status: {}",
                            status_str
                        )));
                    }
                    // Warning status (0xB000 range) - log but don't fail
                    if status >= 0xB000 && status < 0xC000 {
                        tracing::warn!("C-STORE completed with warning status: {}", status_str);
                    }
                }
            }
        }

        Ok(())
    }
}

/// DIMSE operation types used as routing keys for the multiplexing provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DimseOp {
    Find,
    Move,
    Get,
    Store,
}

/// A query provider that multiplexes multiple pipelines on a single SCP listener.
///
/// When several `dicom_scp` endpoints share the same `(bind_addr, port)`, each
/// endpoint typically enables a different subset of DIMSE operations (e.g., one
/// pipeline handles C-FIND while another handles C-STORE). This provider routes
/// each operation to the pipeline that registered for it.
///
/// If multiple endpoints register for the same operation, the first registration
/// wins (a warning is logged during construction).
pub struct MultiplexingQueryProvider {
    /// Maps each DIMSE operation to the pipeline provider that handles it.
    op_routes: HashMap<DimseOp, Arc<PipelineQueryProvider>>,
    /// Fallback provider used when no specific route is registered for an
    /// operation. This is the first provider added, providing backward
    /// compatibility with single-endpoint configurations.
    fallback: Arc<PipelineQueryProvider>,
}

impl MultiplexingQueryProvider {
    /// Build a multiplexing provider from a list of `(pipeline, endpoint, enabled_ops)`.
    ///
    /// `enabled_ops` is derived from the endpoint's `DimseConfig` feature flags
    /// (enable_find, enable_store, etc.). The first endpoint to claim an
    /// operation wins; duplicates are logged as warnings.
    pub fn new(
        routes: Vec<(String, String, Vec<DimseOp>)>,
    ) -> Self {
        assert!(!routes.is_empty(), "MultiplexingQueryProvider requires at least one route");

        let mut op_routes: HashMap<DimseOp, Arc<PipelineQueryProvider>> = HashMap::new();
        let mut fallback: Option<Arc<PipelineQueryProvider>> = None;

        for (pipeline, endpoint, ops) in routes {
            let provider = Arc::new(PipelineQueryProvider::new(
                pipeline.clone(),
                endpoint.clone(),
            ));

            if fallback.is_none() {
                fallback = Some(Arc::clone(&provider));
            }

            for op in ops {
                if let Some(existing) = op_routes.get(&op) {
                    tracing::warn!(
                        "DIMSE operation {:?} already registered by pipeline '{}' endpoint '{}', \
                         ignoring duplicate from pipeline '{}' endpoint '{}'",
                        op,
                        existing.pipeline,
                        existing.endpoint,
                        pipeline,
                        endpoint,
                    );
                } else {
                    tracing::debug!(
                        "DIMSE operation {:?} routed to pipeline '{}' endpoint '{}'",
                        op,
                        pipeline,
                        endpoint,
                    );
                    op_routes.insert(op, Arc::clone(&provider));
                }
            }
        }

        Self {
            op_routes,
            fallback: fallback.expect("at least one route"),
        }
    }

    /// Get the provider for a given operation, falling back to the default.
    fn provider_for(&self, op: DimseOp) -> &PipelineQueryProvider {
        self.op_routes
            .get(&op)
            .map(|p| p.as_ref())
            .unwrap_or(self.fallback.as_ref())
    }
}

#[async_trait]
impl dimse::scp::QueryProvider for MultiplexingQueryProvider {
    async fn find(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
        max_results: u32,
    ) -> DimseResult<Vec<DatasetStream>> {
        self.provider_for(DimseOp::Find)
            .find(query_level, parameters, max_results)
            .await
    }

    async fn locate(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<Vec<DatasetStream>> {
        self.provider_for(DimseOp::Move)
            .locate(query_level, parameters)
            .await
    }

    async fn get(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<Vec<DatasetStream>> {
        self.provider_for(DimseOp::Get)
            .get(query_level, parameters)
            .await
    }

    async fn store(&self, dataset: DatasetStream) -> DimseResult<()> {
        self.provider_for(DimseOp::Store)
            .store(dataset)
            .await
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

    // Note: test_store_dir_functions removed - uses global static that conflicts with parallel test execution

    // ── MultiplexingQueryProvider tests ──────────────────────────────────────

    #[test]
    fn test_multiplexing_provider_routes_to_correct_pipeline() {
        let routes = vec![
            (
                "find_pipeline".into(),
                "find_ep".into(),
                vec![DimseOp::Find],
            ),
            (
                "store_pipeline".into(),
                "store_ep".into(),
                vec![DimseOp::Store],
            ),
        ];
        let mux = MultiplexingQueryProvider::new(routes);

        // Find should route to find_pipeline
        let find_provider = mux.provider_for(DimseOp::Find);
        assert_eq!(find_provider.pipeline, "find_pipeline");
        assert_eq!(find_provider.endpoint, "find_ep");

        // Store should route to store_pipeline
        let store_provider = mux.provider_for(DimseOp::Store);
        assert_eq!(store_provider.pipeline, "store_pipeline");
        assert_eq!(store_provider.endpoint, "store_ep");
    }

    #[test]
    fn test_multiplexing_provider_fallback_for_unregistered_op() {
        let routes = vec![(
            "only_pipeline".into(),
            "only_ep".into(),
            vec![DimseOp::Find],
        )];
        let mux = MultiplexingQueryProvider::new(routes);

        // Get is not registered, should fall back to the first provider
        let get_provider = mux.provider_for(DimseOp::Get);
        assert_eq!(get_provider.pipeline, "only_pipeline");
    }

    #[test]
    fn test_multiplexing_provider_first_registration_wins() {
        let routes = vec![
            (
                "first_pipeline".into(),
                "first_ep".into(),
                vec![DimseOp::Find],
            ),
            (
                "second_pipeline".into(),
                "second_ep".into(),
                vec![DimseOp::Find], // duplicate
            ),
        ];
        let mux = MultiplexingQueryProvider::new(routes);

        let find_provider = mux.provider_for(DimseOp::Find);
        assert_eq!(find_provider.pipeline, "first_pipeline");
    }

    #[test]
    fn test_multiplexing_provider_single_pipeline_all_ops() {
        let routes = vec![(
            "all_in_one".into(),
            "ep".into(),
            vec![DimseOp::Find, DimseOp::Store, DimseOp::Get, DimseOp::Move],
        )];
        let mux = MultiplexingQueryProvider::new(routes);

        for op in [DimseOp::Find, DimseOp::Store, DimseOp::Get, DimseOp::Move] {
            assert_eq!(mux.provider_for(op).pipeline, "all_in_one");
        }
    }
}
