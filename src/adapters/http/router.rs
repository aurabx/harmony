use super::HttpAdapter;
use crate::config::config::Config;
use crate::models::middleware::AuthFailure;
use crate::pipeline::{PipelineError, PipelineExecutor};
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use axum::Router;
use http::{Method, StatusCode};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Build network router for HTTP adapter
///
/// This replaces the old router/dispatcher logic but uses PipelineExecutor
pub async fn build_network_router(config: Arc<Config>, network_name: &str) -> Router {
    let mut app = Router::new();
    let mut route_registry: HashSet<(Method, String)> = HashSet::new();

    for (pipeline_name, pipeline) in &config.pipelines {
        if !pipeline.networks.contains(&network_name.to_string()) {
            continue;
        }

        // Collect routes for this pipeline
        let mut planned: Vec<(String, crate::router::route_config::RouteConfig)> = Vec::new();
        let mut has_conflict = false;

        for endpoint_name in &pipeline.endpoints {
            if let Some(endpoint) = config.endpoints.get(endpoint_name) {
                let service = match endpoint.resolve_service() {
                    Ok(service) => service,
                    Err(err) => {
                        tracing::error!(
                            "Failed to resolve service for endpoint '{}': {}",
                            endpoint_name,
                            err
                        );
                        continue;
                    }
                };

                let opts_map: HashMap<String, serde_json::Value> =
                    endpoint.options.clone().unwrap_or_default();

                // Note: DIMSE SCPs are now started by DimseAdapter in the orchestrator (src/lib.rs)
                // HTTP router no longer launches DIMSE listeners

                let route_configs = service.build_router(&opts_map);

                for route_config in route_configs.clone() {
                    for m in &route_config.methods {
                        let key = (m.clone(), route_config.path.clone());
                        if route_registry.contains(&key) {
                            tracing::warn!(
                                "Dropping pipeline '{}' due to route conflict: {} {}",
                                pipeline_name,
                                m,
                                route_config.path
                            );
                            has_conflict = true;
                            break;
                        }
                    }
                    if has_conflict {
                        break;
                    }
                    planned.push((endpoint_name.clone(), route_config));
                }
                if has_conflict {
                    break;
                }
            }
        }

        if has_conflict {
            continue;
        }

        // Register routes
        for (endpoint_name, route_config) in planned {
            if let Some(endpoint) = config.endpoints.get(&endpoint_name) {
                let path = route_config.path.clone();
                let methods = route_config.methods.clone();

                let mut method_router = axum::routing::MethodRouter::new();
                let mut added_any = false;

                for method in methods.clone() {
                    let key = (method.clone(), path.clone());
                    if route_registry.contains(&key) {
                        tracing::warn!("Skipping duplicate route: {} {}", method, path);
                        continue;
                    }

                    let endpoint_name2 = endpoint_name.clone();
                    let pipeline_name2 = pipeline_name.clone();
                    let config_ref = config.clone();

                    let handler = move |mut req: Request| {
                        let endpoint_name = endpoint_name2.clone();
                        let pipeline_name = pipeline_name2.clone();
                        let config_ref = config_ref.clone();
                        async move {
                            handle_request(&mut req, config_ref, endpoint_name, pipeline_name).await
                        }
                    };

                    method_router = match method {
                        http::Method::GET => method_router.get(handler),
                        http::Method::POST => method_router.post(handler),
                        http::Method::PUT => method_router.put(handler),
                        http::Method::DELETE => method_router.delete(handler),
                        http::Method::PATCH => method_router.patch(handler),
                        http::Method::HEAD => method_router.head(handler),
                        http::Method::OPTIONS => method_router.options(handler),
                        _ => method_router,
                    };

                    route_registry.insert(key);
                    added_any = true;
                }

                if added_any {
                    app = app.route(&path, method_router);
                }
            }
        }

        // Note: DIMSE SCPs (including persistent Store SCPs for backends) are now
        // started by DimseAdapter in the orchestrator (src/lib.rs)
        // HTTP router no longer launches DIMSE listeners
    }

    app
}

/// Handle HTTP request using PipelineExecutor
async fn handle_request(
    req: &mut Request,
    config: Arc<Config>,
    endpoint_name: String,
    pipeline_name: String,
) -> Result<Response<Body>, StatusCode> {
    // Look up the endpoint and pipeline from config
    let endpoint = config
        .endpoints
        .get(&endpoint_name)
        .ok_or(StatusCode::NOT_FOUND)?;

    let service = endpoint
        .resolve_service()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let pipeline = config
        .pipelines
        .get(&pipeline_name)
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    // 1. Convert HTTP Request → ProtocolCtx
    let ctx = HttpAdapter::http_request_to_protocol_ctx(
        req,
        endpoint.options.as_ref().unwrap_or(&HashMap::new()),
    )
    .await
    .map_err(|_| StatusCode::BAD_REQUEST)?;

    // 2. Build envelope via service
    let envelope = service
        .build_protocol_envelope(ctx.clone(), endpoint.options.as_ref().unwrap_or(&HashMap::new()))
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    // 3. Execute pipeline (NEW: using PipelineExecutor!)
    let response_envelope = PipelineExecutor::execute(envelope, pipeline, &config, &ctx)
        .await
        .map_err(|err| map_pipeline_error_to_status(&err))?;

    // 4. Convert ResponseEnvelope → HTTP Response
    let response = service
        .endpoint_outgoing_response(
            response_envelope.clone(),
            endpoint.options.as_ref().unwrap_or(&HashMap::new()),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}

/// Map pipeline errors to HTTP status codes
fn map_pipeline_error_to_status(err: &PipelineError) -> StatusCode {
    match err {
        PipelineError::MiddlewareError(middleware_err) => {
            // Check if it's an AuthFailure
            if let Some(_auth_failure) = middleware_err.downcast_ref::<AuthFailure>() {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
        PipelineError::BackendError(_) => StatusCode::BAD_GATEWAY,
        PipelineError::ConfigError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        PipelineError::ServiceError(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
