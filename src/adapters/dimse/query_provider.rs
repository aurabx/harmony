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
use dimse::types::{dataset_stream_from_vec, BoxDatasetStream, DatasetStream, QueryLevel, RetrieveStream};
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

/// A DICOM Part 10 file encoded for a JSON pipeline body.
#[derive(Debug, PartialEq)]
struct InlineInstance {
    /// The whole Part 10 file in standard base64.
    body_b64: String,
    /// Transfer Syntax UID from the file meta group.
    transfer_syntax_uid: String,
}

impl InlineInstance {
    /// Encode `part10` and read its transfer syntax from the file meta group,
    /// so the stated transfer syntax always matches the bytes.
    fn from_part10(part10: &[u8]) -> DimseResult<Self> {
        use base64::Engine;

        // The file meta group starts at the `DICM` code, after the 128-byte preamble.
        let meta_group = part10.get(128..).ok_or_else(|| {
            DimseError::operation_failed(format!(
                "stored instance is {} bytes, too short for a DICOM Part 10 file",
                part10.len()
            ))
        })?;
        let meta = dicom_object::meta::FileMetaTable::from_reader(meta_group).map_err(|e| {
            DimseError::operation_failed(format!(
                "stored instance has no readable file meta group: {}",
                e
            ))
        })?;

        Ok(Self {
            body_b64: base64::engine::general_purpose::STANDARD.encode(part10),
            transfer_syntax_uid: meta.transfer_syntax().to_string(),
        })
    }
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
    /// Returns the datasets as a stream. Each streamed envelope is converted as
    /// it arrives, so the caller can act on the first dataset while the backend
    /// is still producing the rest.
    async fn run_streaming(
        &self,
        command: Command,
        body: serde_json::Value,
        mut meta: HashMap<String, String>,
    ) -> DimseResult<BoxDatasetStream> {
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

        // Build ProtocolCtx for DIMSE with typed Command.
        // The Command is the single source of truth for the operation.
        meta.insert("protocol".into(), "dimse".into());
        let operation = command.operation;
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

        tracing::info!("Executing streaming pipeline for DIMSE operation: {}", operation);

        // Execute streaming pipeline
        let response_stream =
            PipelineExecutor::execute_streaming(request_envelope, &self.pipeline, pipeline_cfg, &config, &ctx)
                .await
                .map_err(|e| {
                    DimseError::operation_failed(format!("Streaming pipeline execution failed: {}", e))
                })?;

        // Each envelope becomes zero or more datasets as it arrives. Nothing is
        // collected here.
        let datasets = response_stream
            .enumerate()
            .then(|(index, result)| async move {
                let instance = index as u64 + 1;
                match result {
                    Ok(response_envelope) => {
                        tracing::debug!("Processing streamed instance #{}", instance);
                        match Self::response_to_datasets(response_envelope).await {
                            Ok(datasets) => datasets.into_iter().map(Ok).collect::<Vec<_>>(),
                            Err(e) => {
                                tracing::error!(
                                    "Streamed instance #{} could not be converted: {}",
                                    instance, e
                                );
                                vec![Err(e)]
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Stream error on instance #{}: {}", instance, e);
                        vec![Err(DimseError::operation_failed(format!(
                            "Stream failed after {} instances: {}",
                            index, e
                        )))]
                    }
                }
            })
            .flat_map(futures_util::stream::iter);

        Ok(Box::pin(datasets))
    }

    /// Execute a DIMSE operation through a streaming pipeline and collect a single response envelope.
    ///
    /// Some DIMSE operations (notably C-STORE) need to inspect response metadata
    /// set by outgoing middleware, but their pipeline may be backed by a
    /// streaming backend that refuses non-streaming execution. This helper
    /// mirrors `run_streaming` for setup, drives the pipeline via
    /// `PipelineExecutor::execute_streaming`, and consumes the response stream
    /// down to a single envelope. The stream must yield at least one envelope.
    async fn run_streaming_response(
        &self,
        command: Command,
        body: serde_json::Value,
        mut meta: HashMap<String, String>,
    ) -> DimseResult<ResponseEnvelope<Vec<u8>>> {
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

        meta.insert("protocol".into(), "dimse".into());
        let operation = command.operation;
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&body).unwrap_or_default(),
            meta,
            attrs: serde_json::json!({}),
            command: Some(command),
        };

        let request_envelope = service
            .build_protocol_envelope(ctx.clone(), options)
            .await
            .map_err(|e| DimseError::operation_failed(format!("Envelope build failed: {}", e)))?;

        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        tracing::info!("Executing streaming pipeline for DIMSE operation: {} (single-envelope)", operation);

        let mut response_stream =
            PipelineExecutor::execute_streaming(request_envelope, &self.pipeline, pipeline_cfg, &config, &ctx)
                .await
                .map_err(|e| {
                    DimseError::operation_failed(format!("Streaming pipeline execution failed: {}", e))
                })?;

        // Consume the stream and keep the last successful envelope. Middleware
        // (for example dicom_to_dicomweb) may set final response metadata such
        // as dicom_status on the last envelope.
        let mut last: Option<ResponseEnvelope<Vec<u8>>> = None;
        while let Some(item) = response_stream.next().await {
            match item {
                Ok(envelope) => last = Some(envelope),
                Err(e) => {
                    return Err(DimseError::operation_failed(format!(
                        "Streaming pipeline yielded error: {}",
                        e
                    )));
                }
            }
        }

        last.ok_or_else(|| {
            DimseError::operation_failed("Streaming pipeline produced no response envelopes")
        })
    }

    /// Build the pipeline body for one received C-STORE instance.
    ///
    /// `file` is the path of the Part 10 copy on this host. Middleware that
    /// runs in this process (`dicom_to_dicomweb`, `dimse_to_jmix`) reads it.
    /// `inline_instance` adds the instance itself as `body_b64` plus its
    /// `transfer_syntax_uid`, for backends that cannot use a local path.
    fn build_store_body(
        sop_class_uid: Option<&str>,
        dicom_json: Option<serde_json::Value>,
        temp_file: &std::path::Path,
        inline_instance: Option<InlineInstance>,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "command": {
                "message_id": 1,
                "sop_class_uid": sop_class_uid,
                "priority": "MEDIUM",
                "direction": "REQUEST"
            },
            "dataset": dicom_json,
            "file": temp_file.to_string_lossy()
        });
        if let Some(instance) = inline_instance {
            body["body_b64"] = serde_json::Value::String(instance.body_b64);
            body["transfer_syntax_uid"] = serde_json::Value::String(instance.transfer_syntax_uid);
        }
        body
    }

    /// Convert ResponseEnvelope to Vec<DatasetStream>
    ///
    /// Parses the response data and converts it to DICOM datasets.
    /// Uses the envelope_unwrapper module to handle various response formats.
    async fn response_to_datasets(
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
                    match Self::dicom_json_to_dataset(&item) {
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
                let dataset = Self::binary_to_dataset(&bytes)?;
                Ok(vec![dataset])
            }
            UnwrappedContent::FolderPath(folder_path) => {
                tracing::debug!("response_to_datasets: reading DICOM files from folder: {:?}", folder_path);
                Self::folder_to_datasets(&folder_path)
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
    fn dicom_json_to_dataset(json: &serde_json::Value) -> DimseResult<DatasetStream> {
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
    fn binary_to_dataset(bytes: &[u8]) -> DimseResult<DatasetStream> {
        let file_obj = dicom_object::from_reader(bytes).map_err(|e| {
            DimseError::operation_failed(format!("Failed to parse DICOM from binary: {}", e))
        })?;
        Ok(DatasetStream::from_object(file_obj.into_inner()))
    }

    /// Read all DICOM files from a folder and convert to DatasetStreams
    ///
    /// This is used for C-GET/C-MOVE results where files are stored on disk.
    fn folder_to_datasets(folder_path: &std::path::Path) -> DimseResult<Vec<DatasetStream>> {
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
    ) -> DimseResult<BoxDatasetStream> {
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

        let command = Command::new(Operation::Query).with_source("dimse_adapter");
        if PipelineExecutor::is_streaming_pipeline(pipeline_cfg, &config) {
            tracing::info!("Using streaming pipeline for C-FIND");
            self.run_streaming(command, body, meta).await
        } else {
            let response_envelope = self.run(command, body, meta).await?;
            let datasets = Self::response_to_datasets(response_envelope).await?;
            Ok(dataset_stream_from_vec(datasets))
        }
    }

    async fn locate(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<BoxDatasetStream> {
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
        let datasets = Self::response_to_datasets(response_envelope).await?;
        Ok(dataset_stream_from_vec(datasets))
    }

    async fn get(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<RetrieveStream> {
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

        let command = Command::new(Operation::Retrieve).with_source("dimse_adapter");
        if PipelineExecutor::is_streaming_pipeline(pipeline_cfg, &config) {
            tracing::info!("Using streaming pipeline for C-GET");
            // The bridge does not know the instance count up front, so the SCP
            // sends no Pending responses for this retrieval.
            Ok(RetrieveStream::new(self.run_streaming(command, body, meta).await?))
        } else {
            let response_envelope = self.run(command, body, meta).await?;
            let datasets = Self::response_to_datasets(response_envelope).await?;
            Ok(RetrieveStream::from_vec(datasets))
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

        // Execute pipeline and check for DICOM status in response.
        // Dispatch to the streaming executor when the pipeline's backend
        // requires it. This mirrors the pattern used by `find` and `get`.
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;
        let streaming = PipelineExecutor::is_streaming_pipeline(pipeline_cfg, &config);

        // A streaming backend may sit on another host, where `temp_file` means
        // nothing, so it gets the instance itself. The bytes are read back
        // from the file just written so both copies are identical.
        let inline_instance = if streaming {
            let part10 = tokio::fs::read(&temp_file).await.map_err(|e| {
                DimseError::operation_failed(format!(
                    "read stored instance {}: {}",
                    temp_file.display(),
                    e
                ))
            })?;
            Some(InlineInstance::from_part10(&part10)?)
        } else {
            None
        };

        let body = Self::build_store_body(
            ds_meta.sop_class_uid.as_deref(),
            dicom_json,
            &temp_file,
            inline_instance,
        );

        let command = Command::new(Operation::Store).with_source("dimse_adapter");
        let response = if streaming {
            tracing::info!("Using streaming pipeline for C-STORE");
            self.run_streaming_response(command, body, meta).await?
        } else {
            self.run(command, body, meta).await?
        };

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
    ) -> DimseResult<BoxDatasetStream> {
        self.provider_for(DimseOp::Find)
            .find(query_level, parameters, max_results)
            .await
    }

    async fn locate(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<BoxDatasetStream> {
        self.provider_for(DimseOp::Move)
            .locate(query_level, parameters)
            .await
    }

    async fn get(
        &self,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<RetrieveStream> {
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

    #[test]
    fn test_run_streaming_command_carries_correct_operation() {
        // Verify that the Command built at each call site has the expected operation,
        // mirroring what run_streaming() receives in practice.
        let find_cmd = Command::new(Operation::Query).with_source("dimse_adapter");
        assert_eq!(find_cmd.operation, Operation::Query);
        assert_eq!(find_cmd.source.as_deref(), Some("dimse_adapter"));

        let get_cmd = Command::new(Operation::Retrieve).with_source("dimse_adapter");
        assert_eq!(get_cmd.operation, Operation::Retrieve);
        assert_eq!(get_cmd.source.as_deref(), Some("dimse_adapter"));
    }

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

    // ── C-STORE pipeline body ────────────────────────────────────────────────

    fn sample_instance_path() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/dicom/study_1/series_1/CT.1.1.dcm")
    }

    #[test]
    fn test_inline_instance_states_the_transfer_syntax_of_the_file() {
        use base64::Engine;

        let path = sample_instance_path();
        let part10 = std::fs::read(&path).expect("read sample instance");
        let source = dicom_object::open_file(&path).expect("open sample instance");

        let instance = InlineInstance::from_part10(&part10).expect("inline instance");

        assert_eq!(instance.transfer_syntax_uid, source.meta().transfer_syntax());
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&instance.body_b64)
            .expect("body_b64 is base64");
        assert_eq!(decoded, part10);
    }

    #[test]
    fn test_inline_instance_rejects_bytes_that_are_not_part10() {
        for bytes in [vec![0u8; 16], vec![0u8; 200]] {
            let err = InlineInstance::from_part10(&bytes).expect_err("not a Part 10 file");
            assert!(
                err.to_string().contains("stored instance"),
                "unexpected error: {}",
                err
            );
        }
    }

    #[test]
    fn test_store_body_carries_the_instance_only_when_inline() {
        let temp_file = std::path::Path::new("/var/harmony/dimse/instance.dcm");
        let dataset = serde_json::json!({ "00100020": { "vr": "LO", "Value": ["PID1"] } });

        let local = PipelineQueryProvider::build_store_body(
            Some("1.2.840.10008.5.1.4.1.1.2"),
            Some(dataset.clone()),
            temp_file,
            None,
        );
        assert_eq!(local["file"], "/var/harmony/dimse/instance.dcm");
        assert_eq!(local["dataset"], dataset);
        assert_eq!(local["command"]["sop_class_uid"], "1.2.840.10008.5.1.4.1.1.2");
        assert!(local.get("body_b64").is_none());
        assert!(local.get("transfer_syntax_uid").is_none());

        let inline = PipelineQueryProvider::build_store_body(
            Some("1.2.840.10008.5.1.4.1.1.2"),
            Some(dataset),
            temp_file,
            Some(InlineInstance {
                body_b64: "UGFydDEw".to_string(),
                transfer_syntax_uid: "1.2.840.10008.1.2.1".to_string(),
            }),
        );
        assert_eq!(inline["body_b64"], "UGFydDEw");
        assert_eq!(inline["transfer_syntax_uid"], "1.2.840.10008.1.2.1");
        assert_eq!(inline["file"], "/var/harmony/dimse/instance.dcm");
    }

    // ── Streaming bridge hand-off ────────────────────────────────────────────
    //
    // Harmony B turns each retrieved DICOM file into a ResponseEnvelope and
    // writes the stream as NDJSON. Harmony A reads the NDJSON and converts each
    // envelope back into a dataset for the SCP caller. This test runs a real
    // sample instance through all of those stages in process.

    fn bridge_request_details() -> crate::models::envelope::envelope::RequestDetails {
        crate::models::envelope::envelope::RequestDetails {
            method: "POST".to_string(),
            uri: "/stream".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata: HashMap::new(),
            content_metadata: None,
        }
    }

    fn element_text(object: &dicom_object::InMemDicomObject, name: &str) -> String {
        object
            .element_by_name(name)
            .unwrap_or_else(|e| panic!("{} missing: {}", name, e))
            .to_str()
            .unwrap_or_else(|e| panic!("{} is not text: {}", name, e))
            .trim_end_matches(|c: char| c == '\0' || c == ' ')
            .to_string()
    }

    #[tokio::test]
    async fn test_streaming_bridge_round_trip_preserves_instance() {
        use crate::models::services::streaming::StreamMetadata;
        use crate::models::services::types::dicom_scu_streaming::DicomScuStreamingBackend;
        use crate::models::services::types::http_streaming::{HttpStreamingEndpoint, StreamFormat};
        use crate::models::services::types::http_streaming_backend::HttpStreamingBackend;
        use bytes::Bytes;
        use futures_util::StreamExt;

        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let source_path = manifest_dir.join("samples/dicom/study_1/series_1/CT.1.1.dcm");

        // The producer deletes each file after reading it, so give it a copy.
        let tmp_root = manifest_dir.join("tmp");
        std::fs::create_dir_all(&tmp_root).expect("create ./tmp");
        let work_dir = tempfile::tempdir_in(&tmp_root).expect("create work dir");
        let copy_path = work_dir.path().join("instance.dcm");
        std::fs::copy(&source_path, &copy_path).expect("copy sample instance");

        // Harmony B: retrieved file → ResponseEnvelope
        let retrieved: std::pin::Pin<
            Box<dyn futures_util::Stream<Item = Result<DatasetStream, DimseError>> + Send>,
        > = Box::pin(futures_util::stream::iter(vec![Ok(DatasetStream::from_file(
            copy_path, false,
        ))]));
        let envelopes = DicomScuStreamingBackend::convert_dimse_stream_to_envelopes(
            retrieved,
            bridge_request_details(),
            StreamMetadata::new("dimse-get"),
            None,
        );

        // Harmony B: envelopes → NDJSON response body
        let writer = HttpStreamingEndpoint {
            default_format: None,
            buffer_size: None,
            enable_compression: None,
        };
        let response = writer
            .build_streaming_response(Box::pin(envelopes), StreamFormat::Ndjson, &HashMap::new())
            .expect("NDJSON response should build");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read NDJSON body");

        // Harmony A: NDJSON body → ResponseEnvelope. The body arrives in small
        // chunks, as an HTTP client receives it, so the line spans chunk boundaries.
        let chunks: Vec<Result<Bytes, reqwest::Error>> = body
            .chunks(256)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect();
        let parsed: Vec<_> = HttpStreamingBackend::parse_ndjson_stream(
            futures_util::stream::iter(chunks),
            bridge_request_details(),
            None,
        )
        .collect()
        .await;
        assert_eq!(parsed.len(), 1, "one instance in, one envelope out");
        let envelope = parsed
            .into_iter()
            .next()
            .unwrap()
            .expect("NDJSON line should parse");

        // Harmony A SCP: envelope → dataset
        let datasets = PipelineQueryProvider::response_to_datasets(envelope)
            .await
            .expect("envelope should convert to a dataset");
        assert_eq!(datasets.len(), 1, "one envelope in, one dataset out");
        let received = datasets[0].to_object().await.expect("dataset should parse");

        let source = dicom_object::open_file(&source_path).expect("open sample instance");
        for name in ["SOPInstanceUID", "StudyInstanceUID", "SeriesInstanceUID", "SOPClassUID"] {
            assert_eq!(
                element_text(&received, name),
                element_text(&source, name),
                "{} changed in the round trip",
                name
            );
        }
        let source_pixels = source
            .element_by_name("PixelData")
            .expect("source pixel data")
            .to_bytes()
            .expect("source pixel bytes");
        let received_pixels = received
            .element_by_name("PixelData")
            .expect("received pixel data")
            .to_bytes()
            .expect("received pixel bytes");
        assert_eq!(received_pixels, source_pixels, "pixel data changed in the round trip");
    }
}
