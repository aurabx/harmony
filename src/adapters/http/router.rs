use super::HttpAdapter;
use crate::config::config::Config;
use crate::models::mesh::registry::{MeshContext, MeshRegistry};
use crate::models::middleware::{
    AccessDenied, AuthFailure, AuthUnavailable, ContentTypeDenied, MethodDenied, PathDenied,
    RateLimitExceeded,
};
use crate::pipeline::{PipelineError, PipelineExecutor};
use crate::router::route_table::{method_list, RouteLookup, RouteTable};
use arc_swap::ArcSwap;
use axum::body::Body;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use http::StatusCode;
use std::collections::HashMap;
use std::sync::Arc;

/// Build network router for HTTP adapter
///
/// Harmony picks the endpoint for each request itself, from a
/// [`RouteTable`] of every endpoint route on the network. axum only sees one
/// catch-all, so overlapping endpoint paths (a collection and an item below
/// it) cannot make route registration fail. See `router::route_table` for
/// the matching rule.
///
/// Mesh routing takes priority: if a request matches a mesh ingress URL,
/// it is routed to the ingress's endpoint regardless of other route matches.
///
/// Each request runs against the live global config, so a zero-downtime
/// reload reaches the next request. `config` builds the mesh registry, which
/// stays fixed until the adapter restarts; a mesh change restarts every
/// network (see `config::reload`).
pub async fn build_network_router(config: Arc<Config>, network_name: &str) -> Router {
    // Build mesh registry for priority routing (with remote reference resolution)
    let resolver = crate::globals::get_provider_resolver();
    let mesh_registry = Arc::new(
        MeshRegistry::from_config_with_resolver(&config, resolver).await
    );

    tracing::info!(
        "🔧 Building router for network '{}' with {} pipelines",
        network_name,
        config.pipelines.len()
    );

    let routes = Arc::new(LiveRouteTable::new(config, network_name));
    let dispatch = move |req: Request| {
        let routes = routes.clone();
        async move { dispatch_request(req, routes).await }
    };
    let mut app = Router::new()
        .route("/", any(dispatch.clone()))
        .route("/{*path}", any(dispatch));

    // Add mesh routing layer - this runs FIRST before normal route matching
    // If request matches a mesh ingress URL, it's handled here and normal routes are bypassed
    if !mesh_registry.is_empty() {
        let mesh_registry_layer = mesh_registry.clone();

        app = app.layer(axum::middleware::from_fn(move |req: Request, next: axum::middleware::Next| {
            let mesh_registry = mesh_registry_layer.clone();
            async move {
                mesh_routing_layer(req, next, mesh_registry).await
            }
        }));

        tracing::info!("🔗 Mesh routing layer registered (priority routing)");
    }

    app
}

/// The config a request runs against: the one the last reload published.
///
/// `lib::run_with_reload` publishes the config before any adapter starts, so
/// a missing config is a startup bug. The request fails with 500 rather than
/// fall back to a config that may predate a reload.
pub(crate) fn live_config() -> Result<Arc<Config>, StatusCode> {
    crate::globals::get_config().ok_or_else(|| {
        tracing::error!("Request rejected: no config has been published");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// The route table for one network, kept in step with the live config.
///
/// A zero-downtime reload can change endpoint paths and which pipelines run
/// on a network, so the table cannot be fixed when the adapter starts. It is
/// rebuilt the first time a request sees a newly published config. Between
/// reloads the check is one pointer compare.
struct LiveRouteTable {
    network_name: String,
    /// The config the table was built from, and the table.
    current: ArcSwap<(Arc<Config>, Arc<RouteTable>)>,
}

impl LiveRouteTable {
    fn new(config: Arc<Config>, network_name: &str) -> Self {
        let table = Arc::new(build_route_table(&config, network_name));
        Self {
            network_name: network_name.to_string(),
            current: ArcSwap::from_pointee((config, table)),
        }
    }

    /// The table for `config`, rebuilt and cached if `config` is not the one
    /// the cached table came from.
    ///
    /// Requests that straddle a reload may hold different configs and rebuild
    /// in turn. Each still gets the table that matches its own config.
    fn table_for(&self, config: &Arc<Config>) -> Arc<RouteTable> {
        let current = self.current.load();
        if Arc::ptr_eq(&current.0, config) {
            return current.1.clone();
        }
        tracing::info!(
            "🔄 Rebuilding routes for network '{}' from the reloaded config",
            self.network_name
        );
        let table = Arc::new(build_route_table(config, &self.network_name));
        self.current
            .store(Arc::new((config.clone(), table.clone())));
        table
    }
}

/// Build the route table for `network_name` and log what it holds.
fn build_route_table(config: &Config, network_name: &str) -> RouteTable {
    // Config validation rejects these problems at load time. A config that
    // skipped validation still gets a working router without the bad routes.
    let (table, problems) = RouteTable::build(config, network_name);
    for problem in &problems {
        tracing::error!(
            "Route not registered on network '{}': {}",
            network_name,
            problem
        );
    }
    for entry in table.entries() {
        tracing::info!(
            "✅ Registered route: {} [{}] -> endpoint '{}' (pipeline '{}')",
            entry.pattern,
            method_list(&entry.methods),
            entry.owner.endpoint,
            entry.owner.pipeline
        );
    }
    table
}

/// Send one request to the endpoint the route table picks for it.
///
/// The config is read once, so route lookup and the pipeline run see the
/// same config even if a reload lands mid-request.
async fn dispatch_request(mut req: Request, routes: Arc<LiveRouteTable>) -> Response<Body> {
    let config = match live_config() {
        Ok(config) => config,
        Err(status) => return status.into_response(),
    };
    let table = routes.table_for(&config);
    let (endpoint_name, pipeline_name) = match table.lookup(req.method(), req.uri().path()) {
        RouteLookup::Found(entry) => (entry.owner.endpoint.clone(), entry.owner.pipeline.clone()),
        RouteLookup::MethodNotAllowed(allowed) => {
            return (
                StatusCode::METHOD_NOT_ALLOWED,
                [(http::header::ALLOW, method_list(&allowed))],
            )
                .into_response();
        }
        RouteLookup::NotFound => return StatusCode::NOT_FOUND.into_response(),
    };

    handle_request(&mut req, config, endpoint_name, pipeline_name)
        .await
        .into_response()
}

/// Mesh routing middleware layer.
///
/// This runs BEFORE normal route matching. If the request matches a mesh ingress URL,
/// it is handled here and the normal route chain is bypassed.
async fn mesh_routing_layer(
    mut req: Request,
    next: axum::middleware::Next,
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

        let config = match live_config() {
            Ok(config) => config,
            Err(status) => return status.into_response(),
        };

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

    // Check if this is a streaming pipeline
    if PipelineExecutor::is_streaming_pipeline(pipeline, &config) {
        return handle_streaming_request(req, config, endpoint_name, pipeline_name, mesh_context).await;
    }

    // 1. Convert HTTP Request → ProtocolCtx
    let mut ctx = HttpAdapter::http_request_to_protocol_ctx(
        req,
        endpoint.options.as_ref().unwrap_or(&HashMap::new()),
    )
    .await
    .map_err(|err| {
        tracing::error!(
            "Request for pipeline '{}' rejected: could not read the HTTP request: {}",
            pipeline_name,
            err
        );
        StatusCode::BAD_REQUEST
    })?;

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
        .map_err(|err| {
            tracing::error!(
                "Request for pipeline '{}' rejected: endpoint '{}' could not build a request envelope: {}",
                pipeline_name,
                endpoint_name,
                err
            );
            StatusCode::BAD_REQUEST
        })?;

    // 3. Execute pipeline (using standard PipelineExecutor)
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

/// Handle HTTP streaming request using PipelineExecutor::execute_streaming
///
/// When `mesh_context` is Some, the request came through mesh routing and
/// the context is attached to ProtocolCtx, as in `handle_request_with_mesh`.
async fn handle_streaming_request(
    req: &mut Request,
    config: Arc<Config>,
    endpoint_name: String,
    pipeline_name: String,
    mesh_context: Option<MeshContext>,
) -> Result<Response<Body>, StatusCode> {
    use futures_util::StreamExt;

    tracing::info!("Handling streaming request for pipeline: {}", pipeline_name);

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
    .map_err(|err| {
        tracing::error!(
            "Streaming request for pipeline '{}' rejected: could not read the HTTP request: {}",
            pipeline_name,
            err
        );
        StatusCode::BAD_REQUEST
    })?;

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
        .map_err(|err| {
            tracing::error!(
                "Streaming request for pipeline '{}' rejected: endpoint '{}' could not build a request envelope: {}",
                pipeline_name,
                endpoint_name,
                err
            );
            StatusCode::BAD_REQUEST
        })?;

    // 3. Execute streaming pipeline
    let response_stream = PipelineExecutor::execute_streaming(envelope, &pipeline_name, pipeline, &config, &ctx)
        .await
        .map_err(|err| {
            tracing::error!("Streaming pipeline execution failed: {}", err);
            map_pipeline_error_to_status(&err)
        })?;

    // 4. Check if endpoint is http_streaming - if so, use its streaming response builder
    if endpoint.service.to_lowercase() == "http_streaming" {
        // Use http_streaming endpoint to convert stream to HTTP response
        use crate::models::services::types::http_streaming::{HttpStreamingEndpoint, StreamFormat};

        let streaming_endpoint = HttpStreamingEndpoint {
            default_format: None,
            buffer_size: None,
            enable_compression: None,
        };

        // Determine format from options or Accept header
        let empty_options = HashMap::new();
        let options = endpoint.options.as_ref().unwrap_or(&empty_options);
        let format = if let Some(format_value) = options.get("format") {
            StreamFormat::from_config(format_value.as_str().unwrap_or("ndjson"))
        } else {
            // Check Accept header
            let accept = req.headers()
                .get("accept")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");

            if accept.contains("text/event-stream") {
                StreamFormat::Sse
            } else if accept.contains("application/x-ndjson") {
                StreamFormat::Ndjson
            } else if accept.contains("multipart/x-mixed-replace") {
                StreamFormat::Multipart
            } else {
                StreamFormat::Ndjson // Default
            }
        };

        let request_headers = HashMap::new(); // TODO: Extract from req if needed
        let response = streaming_endpoint
            .build_streaming_response(response_stream, format, &request_headers)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok(response)
    } else {
        // For non-streaming endpoints, collect all responses and return as JSON array
        tracing::warn!(
            "Streaming pipeline with non-streaming endpoint '{}' - collecting results",
            endpoint.service
        );

        let results: Vec<_> = response_stream.collect().await;
        let mut response_envelopes = Vec::new();

        for result in results {
            match result {
                Ok(envelope) => response_envelopes.push(envelope),
                Err(e) => {
                    tracing::error!("Stream error: {}", e);
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
        }

        // Convert to JSON array
        let json_array: Vec<serde_json::Value> = response_envelopes
            .iter()
            .filter_map(|env| env.normalized_data.clone())
            .collect();

        let response = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&json_array).unwrap_or_default()))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok(response)
    }
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
            } else if middleware_err.downcast_ref::<AuthUnavailable>().is_some() {
                StatusCode::SERVICE_UNAVAILABLE
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
    fn test_map_auth_unavailable_to_service_unavailable() {
        let pipeline_error = PipelineError::MiddlewareError(Box::new(AuthUnavailable(
            "JWKS fetch failed".to_string(),
        )));

        let status = map_pipeline_error_to_status(&pipeline_error);

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
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
