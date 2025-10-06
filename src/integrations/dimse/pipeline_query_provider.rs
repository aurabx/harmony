use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use crate::router::pipeline_runner::run_pipeline;
use crate::models::envelope::envelope::{RequestEnvelope, RequestDetails};
use crate::globals::get_config;
use dimse::types::{DatasetStream, QueryLevel};
use dimse::{Result as DimseResult};
use dimse::error::DimseError;

pub struct PipelineQueryProvider {
    pipeline: String,
}

impl PipelineQueryProvider {
    pub fn new(pipeline: impl Into<String>) -> Self {
        Self { pipeline: pipeline.into() }
    }

    async fn run(&self, op: &str, body: serde_json::Value, meta: HashMap<String, String>) -> DimseResult<RequestEnvelope<Vec<u8>>> {
        let config = get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;

        let details = RequestDetails {
            method: op.to_string(),
            uri: format!("dicom://scp/{}", op.to_lowercase()),
            headers: HashMap::new(),
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

        let body = serde_json::json!({
            "operation": "find",
            "query_level": format!("{}", query_level),
            "parameters": parameters,
            "max_results": max_results,
        });

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

        let body = serde_json::json!({
            "operation": "move",
            "query_level": format!("{}", query_level),
            "parameters": parameters,
        });

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