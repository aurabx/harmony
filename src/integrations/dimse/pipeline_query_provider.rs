use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use crate::router::pipeline_runner::run_pipeline;
use crate::models::envelope::envelope::{RequestEnvelope, RequestDetails};
use crate::globals::get_config;
use dimse::types::{DatasetStream, QueryLevel};
use dimse::{Result as DimseResult};
use dimse::error::DimseError;
use dicom_json_tool as tool;

pub struct PipelineQueryProvider {
    pipeline: String,
}

impl PipelineQueryProvider {
    pub fn new(pipeline: impl Into<String>) -> Self {
        Self { pipeline: pipeline.into() }
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

    fn build_query_metadata(&self, parameters: &HashMap<String, String>) -> tool::model::QueryMetadata {
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
            out.insert(tag.clone(), tool::model::QueryMetaEntry { match_type: Some(match_type.into()) });
        }
        tool::model::QueryMetadata(out)
    }

    async fn run(&self, op: &str, body: serde_json::Value, meta: HashMap<String, String>) -> DimseResult<RequestEnvelope<Vec<u8>>> {
        let config = get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;

        let details = RequestDetails {
            method: op.to_string(),
            uri: format!("dicom://scp/{}", op.to_lowercase()),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata: meta,
        };

        let envelope = RequestEnvelope {
            request_details: details,
            original_data: serde_json::to_vec(&body).unwrap_or_default(),
            normalized_data: Some(body),
        };

        let processed = run_pipeline(envelope, &self.pipeline, Arc::clone(&config))
            .await
            .map_err(|e| DimseError::operation_failed(format!("Pipeline failed: {}", e)))?;
        Ok(processed)
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
        let cmd = tool::model::CommandMeta { message_id: Some(1), sop_class_uid: None, priority: Some("MEDIUM".into()), direction: Some("REQUEST".into()) };
        let identifier = self.build_identifier_json(parameters);
        let qmeta = self.build_query_metadata(parameters);
        let wrapper = tool::model::Wrapper { command: Some(cmd), identifier, query_metadata: Some(qmeta) };
        let body = serde_json::to_value(&wrapper).map_err(|e| DimseError::operation_failed(format!("Wrapper serialize: {}", e)))?;

        let _envelope = self.run("C-FIND", body, meta).await?;
        // For now, we return empty datasets (stub) and rely on envelope side effects/logging
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

        let cmd = tool::model::CommandMeta { message_id: Some(1), sop_class_uid: None, priority: Some("MEDIUM".into()), direction: Some("REQUEST".into()) };
        let identifier = self.build_identifier_json(parameters);
        let qmeta = self.build_query_metadata(parameters);
        let wrapper = tool::model::Wrapper { command: Some(cmd), identifier, query_metadata: Some(qmeta) };
        let body = serde_json::to_value(&wrapper).map_err(|e| DimseError::operation_failed(format!("Wrapper serialize: {}", e)))?;

        let _envelope = self.run("C-MOVE", body, meta).await?;
        Ok(vec![])
    }

    async fn store(&self, _dataset: DatasetStream) -> DimseResult<()> {
        let mut meta = HashMap::new();
        meta.insert("dicom.operation".into(), "C-STORE".into());
        let body = serde_json::json!({
            "operation": "store"
        });
        let _ = self.run("C-STORE", body, meta).await?;
        Ok(())
    }
}