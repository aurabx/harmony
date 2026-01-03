use crate::config::config::Config;
use axum::{extract::State, response::Json};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Serialize)]
pub struct RouteInfo {
    pub path: String,
    pub methods: Vec<String>,
    pub description: Option<String>,
    pub endpoint_name: String,
    pub service_type: String,
    pub pipeline: String,
}

#[derive(Serialize)]
pub struct RoutesResponse {
    pub routes: Vec<RouteInfo>,
}

pub fn get_routes_info(config: &Config) -> RoutesResponse {
    let mut routes = Vec::new();

    // Iterate through all pipelines to find their routes
    for (pipeline_name, pipeline) in &config.pipelines {
        for endpoint_name in &pipeline.endpoints {
            if let Some(endpoint) = config.endpoints.get(endpoint_name) {
                // Resolve the service to get its route configs
                if let Ok(service) = endpoint.resolve_service() {
                    let default_options = HashMap::new();
                    let options = endpoint.options.as_ref().unwrap_or(&default_options);
                    let route_configs = service.build_router(options);

                    for route_config in route_configs {
                        routes.push(RouteInfo {
                            path: route_config.path.clone(),
                            methods: route_config.methods.iter().map(|m| m.to_string()).collect(),
                            description: route_config.description.clone(),
                            endpoint_name: endpoint_name.clone(),
                            service_type: endpoint.service.clone(),
                            pipeline: pipeline_name.clone(),
                        });
                    }
                }
            }
        }
    }

    // Sort routes by path for consistent output
    routes.sort_by(|a, b| a.path.cmp(&b.path));

    RoutesResponse { routes }
}

pub async fn handle_routes(State(config): State<Arc<Config>>) -> Json<RoutesResponse> {
    Json(get_routes_info(&config))
}
