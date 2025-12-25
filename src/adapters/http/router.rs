use super::HttpAdapter;
use crate::config::config::Config;
use crate::models::mesh::registry::{MeshContext, MeshRegistry};
use crate::models::middleware::{
    AccessDenied, AuthFailure, ContentTypeDenied, MethodDenied, PathDenied, RateLimitExceeded,
};
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
/// This replaces the old router/dispatcher logic but uses PipelineExecutor.
/// Mesh routing takes priority: if a request matches a mesh ingress URL,
/// it is routed to the ingress's endpoint regardless of other route matches.
pub async fn build_network_router(config: Arc<Config>, network_name: &str) -> Router {
    let mut app = Router::new();
    let mut route_registry: HashSet<(Method, String)> = HashSet::new();

    // Build mesh registry for priority routing
    let mesh_registry = Arc::new(MeshRegistry::from_config(&config));
    let mesh_config = config.clone();

    tracing::info!(
        "🔧 Building router for network '{}' with {} pipelines",
        network_name,
        config.pipelines.len()
    );
    for (name, pipeline) in &config.pipelines {
        tracing::debug!(
            "Pipeline '{}': networks={:?}, endpoints={:?}",
            name,
            pipeline.networks,
            pipeline.endpoints
        );
    }

    for (pipeline_name, pipeline) in &config.pipelines {
        if !pipeline.networks.contains(&network_name.to_string()) {
            tracing::debug!(
                "Skipping pipeline '{}' - not on network '{}'",
                pipeline_name,
                network_name
            );
            continue;
        }

        tracing::info!(
            "Processing pipeline '{}' for network '{}'",
            pipeline_name,
            network_name
        );

        // Collect routes for this pipeline
        let mut planned: Vec<(String, crate::router::route_config::RouteConfig)> = Vec::new();
        let mut has_conflict = false;

        for endpoint_name in &pipeline.endpoints {
            tracing::debug!("Looking for endpoint '{}'", endpoint_name);
            if let Some(endpoint) = config.endpoints.get(endpoint_name) {
                tracing::debug!(
                    "Found endpoint '{}' with service '{}'",
                    endpoint_name,
                    endpoint.service
                );
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
                tracing::debug!(
                    "Service '{}' generated {} route configs",
                    endpoint.service,
                    route_configs.len()
                );
                for route_config in &route_configs {
                    tracing::debug!("Route: {} {:?}", route_config.path, route_config.methods);
                }

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
            } else {
                tracing::warn!("Endpoint '{}' not found in config", endpoint_name);
            }
        }

        if has_conflict {
            continue;
        }

        // Register routes
        for (endpoint_name, route_config) in planned {
            if let Some(_endpoint) = config.endpoints.get(&endpoint_name) {
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
                    tracing::info!("✅ Registered route: {}", path);
                    app = app.route(&path, method_router);
                }
            }
        }

        // Note: DIMSE SCPs (including persistent Store SCPs for backends) are now
        // started by DimseAdapter in the orchestrator (src/lib.rs)
        // HTTP router no longer launches DIMSE listeners
    }

    // Add mesh routing layer - this runs FIRST before normal route matching
    // If request matches a mesh ingress URL, it's handled here and normal routes are bypassed
    if !mesh_registry.is_empty() {
        let mesh_registry_layer = mesh_registry.clone();
        let mesh_config_layer = mesh_config.clone();

        app = app.layer(axum::middleware::from_fn(move |req: Request, next: axum::middleware::Next| {
            let mesh_registry = mesh_registry_layer.clone();
            let config = mesh_config_layer.clone();
            async move {
                mesh_routing_layer(req, next, config, mesh_registry).await
            }
        }));

        tracing::info!("🔗 Mesh routing layer registered (priority routing)");
    }

    app
}

/// Mesh routing middleware layer.
///
/// This runs BEFORE normal route matching. If the request matches a mesh ingress URL,
/// it is handled here and the normal route chain is bypassed.
async fn mesh_routing_layer(
    mut req: Request,
    next: axum::middleware::Next,
    config: Arc<Config>,
    mesh_registry: Arc<MeshRegistry>,
) -> Response<Body> {
    use crate::models::mesh::config::IngressEgressMode;

    // Extract URL components for mesh matching
    let (scheme, host, port, path) = extract_request_url_parts(&req);

    // Check mesh registry for a match
    if let Some(route_match) = mesh_registry.resolve(&scheme, &host, port, &path) {
        // Check if ingress mode=mesh but no mesh was matched
        // (mesh_name is empty when ingress isn't part of any enabled mesh)
        if route_match.context.ingress.mode == IngressEgressMode::Mesh 
            && route_match.context.mesh_name.is_empty() 
        {
            tracing::warn!(
                "🚫 Mesh route rejected: ingress '{}' requires mesh auth but no mesh matched",
                route_match.context.ingress_name
            );
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::empty())
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::empty())
                        .unwrap()
                });
        }

        tracing::info!(
            "🔗 Mesh route: {}://{}{} -> endpoint '{}' (mesh '{}')",
            scheme,
            host,
            path,
            route_match.endpoint_name,
            route_match.context.mesh_name
        );

        // Handle via mesh routing - bypasses normal routes
        return match handle_request_with_mesh(
            &mut req,
            config,
            route_match.endpoint_name,
            route_match.pipeline_name,
            Some(route_match.context),
        )
        .await
        {
            Ok(response) => response,
            Err(status) => Response::builder()
                .status(status)
                .body(Body::empty())
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::empty())
                        .unwrap()
                }),
        };
    }

    // No mesh match - continue to normal routing
    next.run(req).await
}

/// Extract URL components from request for mesh matching.
fn extract_request_url_parts(req: &Request) -> (String, String, Option<u16>, String) {
    // Determine scheme from request
    let scheme = req
        .uri()
        .scheme_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // Fallback: check X-Forwarded-Proto or assume http
            req.headers()
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "http".to_string())
        });

    // Get host from Host header or URI
    let host_header = req
        .headers()
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Parse host and port from Host header (may include port)
    let (host, port) = if let Some(colon_pos) = host_header.rfind(':') {
        let potential_port = &host_header[colon_pos + 1..];
        if let Ok(p) = potential_port.parse::<u16>() {
            (host_header[..colon_pos].to_string(), Some(p))
        } else {
            (host_header.to_string(), None)
        }
    } else {
        (host_header.to_string(), None)
    };

    let path = req.uri().path().to_string();

    (scheme, host, port, path)
}

/// Handle HTTP request using PipelineExecutor
async fn handle_request(
    req: &mut Request,
    config: Arc<Config>,
    endpoint_name: String,
    pipeline_name: String,
) -> Result<Response<Body>, StatusCode> {
    handle_request_with_mesh(req, config, endpoint_name, pipeline_name, None).await
}

/// Handle HTTP request with optional mesh context.
///
/// When `mesh_context` is Some, the request came through mesh routing and
/// the context is attached to ProtocolCtx for downstream middleware.
async fn handle_request_with_mesh(
    req: &mut Request,
    config: Arc<Config>,
    endpoint_name: String,
    pipeline_name: String,
    mesh_context: Option<MeshContext>,
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
    let mut ctx = HttpAdapter::http_request_to_protocol_ctx(
        req,
        endpoint.options.as_ref().unwrap_or(&HashMap::new()),
    )
    .await
    .map_err(|_| StatusCode::BAD_REQUEST)?;

    // 1b. Attach mesh context if present
    if let Some(ref mesh_ctx) = mesh_context {
        attach_mesh_context(&mut ctx, mesh_ctx);
    }

    // 2. Build envelope via service
    let envelope = service
        .build_protocol_envelope(
            ctx.clone(),
            endpoint.options.as_ref().unwrap_or(&HashMap::new()),
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    // 3. Execute pipeline (NEW: using PipelineExecutor!)
    let response_envelope = PipelineExecutor::execute(envelope, &pipeline_name, pipeline, &config, &ctx)
        .await
        .map_err(|err| {
            tracing::error!("Pipeline execution failed: {}", err);
            map_pipeline_error_to_status(&err)
        })?;

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

/// Attach mesh context to ProtocolCtx for downstream middleware.
fn attach_mesh_context(ctx: &mut crate::models::protocol::ProtocolCtx, mesh_ctx: &MeshContext) {
    // Add mesh info to meta
    ctx.meta.insert("mesh_name".to_string(), mesh_ctx.mesh_name.clone());
    ctx.meta.insert("mesh_ingress".to_string(), mesh_ctx.ingress_name.clone());
    ctx.meta.insert("mesh_pipeline".to_string(), mesh_ctx.pipeline_name.clone());
    ctx.meta.insert("mesh_provider".to_string(), mesh_ctx.mesh.provider.to_string());
    ctx.meta.insert("mesh_type".to_string(), mesh_ctx.mesh.mesh_type.to_string());

    // Add structured mesh context to attrs
    if let serde_json::Value::Object(ref mut attrs) = ctx.attrs {
        attrs.insert(
            "mesh".to_string(),
            serde_json::json!({
                "name": mesh_ctx.mesh_name,
                "ingress": mesh_ctx.ingress_name,
                "pipeline": mesh_ctx.pipeline_name,
                "provider": mesh_ctx.mesh.provider.to_string(),
                "type": mesh_ctx.mesh.mesh_type.to_string(),
                "endpoint": mesh_ctx.ingress.endpoint,
            }),
        );
    }
}

/// Map pipeline errors to HTTP status codes
pub(crate) fn map_pipeline_error_to_status(err: &PipelineError) -> StatusCode {
    match err {
        PipelineError::MiddlewareError(middleware_err) => {
            // Check for specific middleware error types and map to appropriate status codes
            if middleware_err.downcast_ref::<AuthFailure>().is_some() {
                StatusCode::UNAUTHORIZED
            } else if middleware_err.downcast_ref::<PathDenied>().is_some() {
                StatusCode::NOT_FOUND
            } else if middleware_err.downcast_ref::<MethodDenied>().is_some() {
                StatusCode::METHOD_NOT_ALLOWED
            } else if middleware_err.downcast_ref::<ContentTypeDenied>().is_some() {
                StatusCode::UNSUPPORTED_MEDIA_TYPE
            } else if middleware_err.downcast_ref::<RateLimitExceeded>().is_some() {
                StatusCode::TOO_MANY_REQUESTS
            } else if middleware_err.downcast_ref::<AccessDenied>().is_some() {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
        PipelineError::BackendError(_) => StatusCode::BAD_GATEWAY,
        PipelineError::ConfigError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        PipelineError::ServiceError(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::middleware::AuthFailure;
    use crate::pipeline::PipelineError;
    use http::StatusCode;

    #[test]
    fn test_map_auth_failure_to_unauthorized() {
        let auth_error = AuthFailure("Invalid token");
        let pipeline_error = PipelineError::MiddlewareError(Box::new(auth_error));

        let status = map_pipeline_error_to_status(&pipeline_error);

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_map_generic_middleware_error_to_internal_server_error() {
        let generic_error = std::io::Error::new(std::io::ErrorKind::Other, "Generic error");
        let pipeline_error = PipelineError::MiddlewareError(Box::new(generic_error));

        let status = map_pipeline_error_to_status(&pipeline_error);

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_map_backend_error_to_bad_gateway() {
        let pipeline_error = PipelineError::BackendError("Backend timeout".to_string());

        let status = map_pipeline_error_to_status(&pipeline_error);

        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn test_map_config_error_to_internal_server_error() {
        let pipeline_error = PipelineError::ConfigError("Invalid pipeline config".to_string());

        let status = map_pipeline_error_to_status(&pipeline_error);

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_map_service_error_to_internal_server_error() {
        let pipeline_error = PipelineError::ServiceError("Service failed".to_string());

        let status = map_pipeline_error_to_status(&pipeline_error);

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_all_error_variants_covered() {
        // This test ensures we handle all PipelineError variants
        // If a new variant is added, this will fail to compile
        let auth_error = PipelineError::MiddlewareError(Box::new(AuthFailure("test")));
        let backend_error = PipelineError::BackendError("test".to_string());
        let config_error = PipelineError::ConfigError("test".to_string());
        let service_error = PipelineError::ServiceError("test".to_string());

        // Verify all map to valid status codes
        assert!(map_pipeline_error_to_status(&auth_error).is_client_error());
        assert!(map_pipeline_error_to_status(&backend_error).is_server_error());
        assert!(map_pipeline_error_to_status(&config_error).is_server_error());
        assert!(map_pipeline_error_to_status(&service_error).is_server_error());
    }

    #[test]
    fn test_error_status_code_values() {
        // Verify exact status code values
        let auth_error = PipelineError::MiddlewareError(Box::new(AuthFailure("test")));
        assert_eq!(map_pipeline_error_to_status(&auth_error).as_u16(), 401);

        let backend_error = PipelineError::BackendError("test".to_string());
        assert_eq!(map_pipeline_error_to_status(&backend_error).as_u16(), 502);

        let config_error = PipelineError::ConfigError("test".to_string());
        assert_eq!(map_pipeline_error_to_status(&config_error).as_u16(), 500);

        let service_error = PipelineError::ServiceError("test".to_string());
        assert_eq!(map_pipeline_error_to_status(&service_error).as_u16(), 500);
    }

    #[test]
    fn test_pipeline_error_display() {
        // Test error message formatting
        let backend_error = PipelineError::BackendError("Connection refused".to_string());
        assert_eq!(
            backend_error.to_string(),
            "Backend error: Connection refused"
        );

        let config_error = PipelineError::ConfigError("Missing pipeline".to_string());
        assert_eq!(config_error.to_string(), "Config error: Missing pipeline");

        let service_error = PipelineError::ServiceError("Service unavailable".to_string());
        assert_eq!(
            service_error.to_string(),
            "Service error: Service unavailable"
        );
    }

    #[test]
    fn test_pipeline_error_from_string() {
        let error: PipelineError = "Test error".into();
        assert!(matches!(error, PipelineError::ServiceError(_)));
        assert_eq!(error.to_string(), "Service error: Test error");

        let error: PipelineError = "Another error".to_string().into();
        assert!(matches!(error, PipelineError::ServiceError(_)));
        assert_eq!(error.to_string(), "Service error: Another error");
    }
}
