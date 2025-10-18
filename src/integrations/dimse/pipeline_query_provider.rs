use crate::globals::get_config;
use crate::models::envelope::envelope::ResponseEnvelope;
use crate::pipeline::executor::PipelineExecutor;
use async_trait::async_trait;
use dicom_json_tool as tool;
use dimse::error::DimseError;
use dimse::types::{DatasetStream, QueryLevel};
use dimse::Result as DimseResult;
use std::collections::HashMap;

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
        
        // TODO(Phase 3C): Map ResponseEnvelope to C-FIND datasets
        // - Extract normalized_data from response
        // - Convert JSON results to DICOM datasets
        // - Stream multiple Pending responses for multi-match
        // - Send final Success status
        // For now, log the response and return empty (stub)
        tracing::debug!(
            "C-FIND response status: {}, payload size: {} bytes",
            response_envelope.response_details.status,
            response_envelope.original_data.len()
        );
        
        Ok(vec![])
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
        
        // TODO(Phase 3C): Map ResponseEnvelope to C-MOVE progress
        // - Parse response for move progress (remaining/completed sub-ops)
        // - Stream Pending responses with counters
        // - Map pipeline errors to DIMSE failure statuses
        // - Send final Success/Failure status
        // For now, log the response and return empty (stub)
        tracing::debug!(
            "C-MOVE response status: {}, payload size: {} bytes",
            response_envelope.response_details.status,
            response_envelope.original_data.len()
        );
        
        Ok(vec![])
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
        
        // TODO(Phase 3C): Use ResponseEnvelope to determine C-STORE status
        // - Check response_envelope.response_details.status
        // - Map 2xx → DIMSE Success (0x0000)
        // - Map 4xx/5xx → appropriate DIMSE failure statuses
        // - Return status via DimseResult
        match self.run("C-STORE", body, meta).await {
            Ok(response) => {
                tracing::debug!(
                    "C-STORE pipeline response: status={}",
                    response.response_details.status
                );
                // For now, accept all pipeline responses as success
                Ok(())
            }
            Err(e) => {
                tracing::error!("C-STORE pipeline failed: {}", e);
                Err(e)
            }
        }
    }
}
