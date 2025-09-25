mod dispatcher;
pub mod route_config;

use std::sync::Arc;
use axum::Router;
use crate::config::config::Config;
use crate::router::dispatcher::Dispatcher;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
}

pub async fn build_network_router(
    config: Arc<Config>,
    network_name: &str,
) -> Router<()> {
    let dispatcher = Dispatcher::new(config.clone());

    let mut app = Router::new();

    for (group_name, group) in &config.pipelines {
        if !group.networks.contains(&network_name.to_string()) {
            continue;
        }
        app = dispatcher.build_router(app, group);
    }

    app
}

