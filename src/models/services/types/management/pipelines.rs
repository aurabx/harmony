use crate::models::pipelines::config::Pipeline;
use axum::{extract::State, response::Json};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Serialize)]
pub struct PipelineInfo {
    pub id: String,
    pub description: String,
    pub networks: Vec<String>,
    pub endpoints: Vec<String>,
    pub backends: Vec<String>,
    pub middleware: Vec<String>,
}

#[derive(Serialize)]
pub struct PipelinesResponse {
    pub pipelines: Vec<PipelineInfo>,
}

pub fn get_pipelines_info(pipelines: &HashMap<String, Pipeline>) -> PipelinesResponse {
    let pipelines = pipelines
        .iter()
        .map(|(id, pipeline)| PipelineInfo {
            id: id.clone(),
            description: pipeline.description.clone(),
            networks: pipeline.networks.clone(),
            endpoints: pipeline.endpoints.clone(),
            backends: pipeline.backends.clone(),
            middleware: pipeline.middleware.clone(),
        })
        .collect();

    PipelinesResponse { pipelines }
}

pub async fn handle_pipelines(
    State(pipelines): State<Arc<HashMap<String, Pipeline>>>,
) -> Json<PipelinesResponse> {
    Json(get_pipelines_info(&pipelines))
}
