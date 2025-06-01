use axum::{
    routing::{get},  // Add post to imports
    Router,
    Json,
};

use crate::config::{Config, EndpointKind};
use crate::middleware::build_middleware_stack;

pub async fn build_router(config: &Config) -> Router {
    let mut app = Router::new();

    // Configure endpoints based on the config
    for (_name, endpoint) in &config.endpoints {
        let route = match &endpoint.kind {
            EndpointKind::Fhir => {
                Router::new()
                    .route("/*path", get(handle_fhir_request).post(handle_fhir_request))
            },
            EndpointKind::Jdx => {
                Router::new()
                    .route("/*path", get(handle_jdx_request).post(handle_jdx_request))
            },
            EndpointKind::Basic => {
                Router::new()
                    .route("/", get(handle_basic_request).post(handle_basic_request))
                    .route("/*path", get(handle_basic_request).post(handle_basic_request))
            },
            _ => continue,
        };

        // Add configured middleware if any
        let route = if let Some(middleware_list) = &endpoint.middleware {
            build_middleware_stack(route, middleware_list, &config.middleware)
        } else {
            route
        };

        // Mount the route at the configured path prefix
        app = app.nest(&endpoint.path_prefix, route);
    }

    app
}

async fn handle_basic_request() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "message": "Basic endpoint responding"
    }))
}

async fn handle_fhir_request(
    // Add proper types for request extraction
) -> impl axum::response::IntoResponse {
    // Implement FHIR request handling
    axum::response::Json(serde_json::json!({
        "status": "not_implemented",
        "message": "FHIR endpoint handler"
    }))
}

async fn handle_jdx_request(
    // Add proper types for request extraction
) -> impl axum::response::IntoResponse {
    // Implement JDX request handling
    axum::response::Json(serde_json::json!({
        "status": "not_implemented",
        "message": "JDX endpoint handler"
    }))
}