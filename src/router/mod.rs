pub mod route_config;

// Phase 6 cleanup complete (2025-10-18):
// - dispatcher.rs: Deleted (use HttpAdapter instead)
// - pipeline_runner.rs: Deleted (DIMSE now uses PipelineExecutor)
// - scp_launcher.rs: Deleted (DimseAdapter started by orchestrator in src/lib.rs)

use crate::config::config::Config;
use axum::Router;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
}

/// Build network router for HTTP endpoints
///
/// This function now delegates to the HttpAdapter for actual routing.
/// The old dispatcher-based approach is deprecated.
pub async fn build_network_router(config: Arc<Config>, network_name: &str) -> Router<()> {
    // Delegate to HttpAdapter's router builder
    crate::adapters::http::router::build_network_router(config, network_name).await
}
