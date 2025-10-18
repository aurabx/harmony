#[deprecated(
    since = "0.2.0",
    note = "Dispatcher is deprecated. Use adapters::http::router::build_network_router instead. Will be removed after Phase 6."
)]
mod dispatcher;

pub mod pipeline_runner;
pub mod route_config;
pub mod scp_launcher;

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
