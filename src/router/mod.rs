mod dispatcher;
pub mod pipeline_runner;
pub mod route_config;
pub mod scp_launcher;

use crate::config::config::Config;
use crate::router::dispatcher::Dispatcher;
use axum::Router;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
}

pub async fn build_network_router(config: Arc<Config>, network_name: &str) -> Router<()> {
    let dispatcher = Dispatcher::new(config.clone());

    let mut app = Router::new();

    for (group_name, group) in &config.pipelines {
        if !group.networks.contains(&network_name.to_string()) {
            continue;
        }
        app = dispatcher.build_router(app, group_name, group);
    }

    app
}
