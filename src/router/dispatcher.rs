use crate::config::config::Config;
use crate::models::backends::backends::Backend;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::chain::MiddlewareChain;
use crate::models::middleware::middleware::build_middleware_instances_for_pipeline;
use crate::models::pipelines::config::Pipeline;
use axum::body::Body;
use axum::extract::Request;
use axum::{response::Response, Router};
use http::{Method, StatusCode};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct Dispatcher {
    config: Arc<Config>,
}

impl Dispatcher {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Builds incoming routes for a specific group within the given app router.
    pub fn build_router(
        &self,
        mut app: Router<()>,
        group_name: &str,
        group: &Pipeline,
        route_registry: &mut HashSet<(Method, String)>,
    ) -> Router<()> {
        // Preflight: collect all planned (method, path) for this group and detect conflicts
        let mut planned: Vec<(String, crate::router::route_config::RouteConfig)> = Vec::new();
        let mut has_conflict = false;
        // Track DICOM SCP endpoints (which have no HTTP routes) so we can start listeners
        let mut scp_endpoints: Vec<(String, HashMap<String, serde_json::Value>)> = Vec::new();

        for endpoint_name in &group.endpoints {
            if let Some(endpoint) = self.config.endpoints.get(endpoint_name) {
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
                // Detect DICOM SCP endpoints (not backends)
                if endpoint.service.eq_ignore_ascii_case("dicom") {
                    let is_backend = opts_map.contains_key("host") || opts_map.contains_key("aet");
                    if !is_backend {
                        scp_endpoints.push((endpoint_name.clone(), opts_map.clone()));
                    }
                }

                let route_configs = service.build_router(&opts_map);

                for route_config in route_configs.clone() {
                    for m in &route_config.methods {
                        let key = (m.clone(), route_config.path.clone());
                        if route_registry.contains(&key) {
                            tracing::warn!(
                                "Dropping pipeline '{}' due to route conflict: {} {}",
                                group_name,
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
            // Skip registering any routes for this group
            return app;
        }

        // Ensure any DICOM SCP listeners are started, even if no routes are registered
        for (ep_name, opts) in scp_endpoints.iter() {
            // Ensure a storage_dir for the SCP based on configured storage adapter if not provided
            let mut opts_with_storage = opts.clone();
            if !opts_with_storage.contains_key("storage_dir") {
                let storage_root = self
                    .config
                    .storage
                    .options
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("./tmp");
                let dimse_root = std::path::Path::new(storage_root).join("dimse");
                opts_with_storage.insert(
                    "storage_dir".to_string(),
                    serde_json::json!(dimse_root.to_string_lossy().to_string()),
                );
            }
            crate::router::scp_launcher::ensure_dimse_scp_started(
                ep_name,
                group_name,
                &opts_with_storage,
            );
        }

        // No conflicts: register routes and start any listeners
        for (endpoint_name, route_config) in planned {
            if let Some(endpoint) = self.config.endpoints.get(&endpoint_name) {
                // Start DICOM SCP for endpoint (SCP mode) — keep for safety; guarded by registry
                if endpoint.service.eq_ignore_ascii_case("dicom") {
                    let opts_map: HashMap<String, serde_json::Value> =
                        endpoint.options.clone().unwrap_or_default();
                    let is_backend = opts_map.contains_key("host") || opts_map.contains_key("aet");
                    if !is_backend {
                        crate::router::scp_launcher::ensure_dimse_scp_started(
                            &endpoint_name,
                            group_name,
                            &opts_map,
                        );
                    }
                }

                let path = route_config.path.clone();
                let methods = route_config.methods.clone();

                let mut method_router = axum::routing::MethodRouter::new();
                let mut added_any = false;

                for method in methods.clone() {
                    let key = (method.clone(), path.clone());
                    if route_registry.contains(&key) {
                        // Shouldn't happen due to preflight, but guard anyway
                        tracing::warn!("Skipping duplicate route: {} {}", method, path);
                        continue;
                    }

                    let endpoint_name2 = endpoint_name.clone();
                    let config_ref = self.config.clone();

                    let handler = move |mut req: Request| {
                        let endpoint_name = endpoint_name2.clone();
                        let config_ref = config_ref.clone();
                        async move {
                            Dispatcher::handle_request_async(&mut req, config_ref, endpoint_name)
                                .await
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

        // If requested, start a persistent Store SCP for any DICOM backends in this pipeline
        for backend_name in &group.backends {
            if let Some(backend) = self.config.backends.get(backend_name) {
                if backend.service.eq_ignore_ascii_case("dicom") {
                    let mut opts = backend.options.clone().unwrap_or_default();
                    let persistent = opts
                        .get("persistent_store_scp")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if persistent {
                        // Map incoming_store_port -> port for SCP launcher
                        if let Some(p) = opts.get("incoming_store_port").and_then(|v| v.as_u64()) {
                            opts.insert("port".to_string(), serde_json::json!(p as u16));
                        }
                        // Ensure local_aet exists (fallback matches SCU default)
                        if !opts.contains_key("local_aet") {
                            opts.insert("local_aet".to_string(), serde_json::json!("HARMONY_SCU"));
                        }
                        // Ensure storage_dir is provided from storage adapter configuration
                        if !opts.contains_key("storage_dir") {
                            let storage_root = self
                                .config
                                .storage
                                .options
                                .get("path")
                                .and_then(|v| v.as_str())
                                .unwrap_or("./tmp");
                            let dimse_root = std::path::Path::new(storage_root).join("dimse");
                            opts.insert(
                                "storage_dir".to_string(),
                                serde_json::json!(dimse_root.to_string_lossy().to_string()),
                            );
                        }
                        crate::router::scp_launcher::ensure_dimse_scp_started(
                            backend_name,
                            group_name,
                            &opts,
                        );
                    }
                }
            }
        }

        app
    }

    async fn handle_request_async(
        req: &mut Request,
        config: Arc<Config>,
        endpoint_name: String,
    ) -> Result<Response<Body>, StatusCode> {
        // Look up the endpoint and group from config
        let endpoint = config
            .endpoints
            .get(&endpoint_name)
            .ok_or(StatusCode::NOT_FOUND)?;

        let service = endpoint
            .resolve_service()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let pipeline = config
            .pipelines
            .iter()
            .find(|(_, g)| g.endpoints.contains(&endpoint_name))
            .map(|(_, g)| g)
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        // Prefer protocol-agnostic envelope builder; fallback to HTTP-only if not implemented
        let ctx = Self::http_request_to_protocol_ctx(
            req,
            endpoint.options.as_ref().unwrap_or(&HashMap::new()),
        )
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

        let envelope = service
            .build_protocol_envelope(ctx, endpoint.options.as_ref().unwrap_or(&HashMap::new()))
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        // 1. Process through endpoint service
        let processed_envelope = service
            .transform_request(
                envelope,
                endpoint.options.as_ref().unwrap_or(&HashMap::new()),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // 2. Incoming (left) middleware chain
        let after_incoming_mw =
            Self::process_incoming_middleware(processed_envelope, pipeline, &config)
                .await
                .map_err(|err| {
                    tracing::warn!("Incoming middleware failed: {:?}", err);
                    StatusCode::UNAUTHORIZED
                })?;

        // 3. Process through configured backends
        let after_backends = Self::process_backends(after_incoming_mw, pipeline, &config)
            .await
            .map_err(|err| {
                tracing::error!("Backend processing failed: {:?}", err);
                StatusCode::BAD_GATEWAY
            })?;

        // 4. Outgoing (right) middleware chain
        let after_outgoing_mw =
            Self::process_outgoing_middleware(after_backends, pipeline, &config)
                .await
                .map_err(|err| {
                    tracing::error!("Outgoing middleware failed: {:?}", err);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

        // 5. Final endpoint response processing
        let response = service
            .transform_response(
                after_outgoing_mw.clone(),
                endpoint.options.as_ref().unwrap_or(&HashMap::new()),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Return the Response produced by the service directly (service controls body/headers)
        Ok(response)
    }

    // Convert an Axum HTTP request into a ProtocolCtx for protocol-agnostic envelope builders
    async fn http_request_to_protocol_ctx(
        req: &mut Request,
        options: &HashMap<String, serde_json::Value>,
    ) -> Result<crate::models::protocol::ProtocolCtx, crate::utils::Error> {
        use crate::models::protocol::{Protocol, ProtocolCtx};
        use crate::utils::Error;
        use axum::body::Body;

        // Compute subpath using path_prefix option
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let path_only = req.uri().path().to_string();
        let full_path_with_query = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| path_only.clone());
        let mut subpath = path_only
            .strip_prefix(path_prefix)
            .unwrap_or("")
            .to_string();
        if subpath.starts_with('/') {
            subpath = subpath.trim_start_matches('/').to_string();
        }

        // Headers
        let headers_obj: serde_json::Value = {
            let map: serde_json::Map<String, serde_json::Value> = req
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        serde_json::Value::String(v.to_str().unwrap_or_default().to_string()),
                    )
                })
                .collect();
            serde_json::Value::Object(map)
        };

        // Cookies
        let cookies_obj: serde_json::Value = {
            let mut map = serde_json::Map::new();
            for val in req.headers().get_all(http::header::COOKIE).iter() {
                if let Ok(s) = val.to_str() {
                    for part in s.split(';') {
                        let kv = part.trim();
                        if kv.is_empty() {
                            continue;
                        }
                        let mut split = kv.splitn(2, '=');
                        let name = split.next().unwrap_or("").trim();
                        let value = split.next().unwrap_or("").trim();
                        if !name.is_empty() {
                            map.insert(
                                name.to_string(),
                                serde_json::Value::String(value.to_string()),
                            );
                        }
                    }
                }
            }
            serde_json::Value::Object(map)
        };

        // Query params
        let query_obj: serde_json::Value = {
            let mut root = serde_json::Map::new();
            if let Some(q) = req.uri().query() {
                for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
                    root.entry(k.to_string())
                        .or_insert_with(|| serde_json::Value::Array(vec![]))
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::Value::String(v.to_string()));
                }
            }
            serde_json::Value::Object(root)
        };

        // Cache status
        let cache_status = req
            .headers()
            .get("Cache-Status")
            .or_else(|| req.headers().get("X-Cache"))
            .or_else(|| req.headers().get("CF-Cache-Status"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        // Metadata
        let mut meta_map = std::collections::HashMap::new();
        meta_map.insert("protocol".to_string(), "http".to_string());
        meta_map.insert("path".to_string(), subpath);
        meta_map.insert("full_path".to_string(), full_path_with_query);

        // attrs object
        let mut attrs = serde_json::Map::new();
        attrs.insert(
            "method".to_string(),
            serde_json::Value::String(req.method().to_string()),
        );
        attrs.insert(
            "uri".to_string(),
            serde_json::Value::String(req.uri().to_string()),
        );
        attrs.insert("headers".to_string(), headers_obj);
        attrs.insert("cookies".to_string(), cookies_obj);
        attrs.insert("query_params".to_string(), query_obj);
        attrs.insert(
            "cache_status".to_string(),
            serde_json::Value::String(cache_status),
        );

        // Body bytes
        let body_bytes =
            axum::body::to_bytes(std::mem::replace(req.body_mut(), Body::empty()), usize::MAX)
                .await
                .map_err(|_| Error::from("Failed to read request body"))?
                .to_vec();

        Ok(ProtocolCtx {
            protocol: Protocol::Http,
            payload: body_bytes,
            meta: meta_map,
            attrs: serde_json::Value::Object(attrs),
        })
    }

    // Process through backend(s)
    async fn process_backends(
        mut envelope: RequestEnvelope<Vec<u8>>,
        group: &Pipeline,
        config: &Config,
    ) -> Result<RequestEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        // Allow endpoints to short-circuit backend processing by setting a metadata flag
        if envelope
            .request_details
            .metadata
            .get("skip_backends")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            tracing::info!("Skipping backends due to endpoint 'skip_backends' flag");
            return Ok(envelope);
        }

        tracing::info!("Processing through {} backends", group.backends.len());

        // Process each backend in the group
        for backend_name in &group.backends {
            if let Some(backend) = config.backends.get(backend_name) {
                envelope = Self::process_single_backend(envelope, backend).await?;
            } else {
                tracing::warn!("Backend '{}' not found in config", backend_name);
            }
        }

        Ok(envelope)
    }

    // Process through a single backend
    async fn process_single_backend(
        mut envelope: RequestEnvelope<Vec<u8>>,
        backend: &Backend,
    ) -> Result<RequestEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let service = backend
            .resolve_service()
            .map_err(|err| format!("Failed to resolve backend service: {}", err))?;

        // Transform the request using the backend service
        envelope = service
            .transform_request(
                envelope,
                backend.options.as_ref().unwrap_or(&HashMap::new()),
            )
            .await
            .map_err(|err| format!("Backend request transformation failed: {:?}", err))?;

        tracing::info!(
            "Processed backend '{}' using service type '{}'",
            backend.service,
            backend.service
        );

        Ok(envelope)
    }

    // Process through incoming middleware chain
    async fn process_incoming_middleware(
        envelope: RequestEnvelope<Vec<u8>>,
        group: &Pipeline,
        config: &Config,
    ) -> Result<RequestEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        // Clone normalized_data before using it to avoid ownership issues
        let normalized_data = envelope.normalized_data.clone();

        // Convert envelope to use serde_json::Value for middleware processing
        let json_envelope = RequestEnvelope {
            request_details: envelope.request_details.clone(),
            original_data: normalized_data.unwrap_or_else(|| {
                serde_json::from_slice(&envelope.original_data).unwrap_or(serde_json::Value::Null)
            }),
            normalized_data: envelope.normalized_data.clone(),
            normalized_snapshot: envelope.normalized_snapshot.clone(),
        };

        // Build middleware instances from pipeline names
        let middleware_instances = build_middleware_instances_for_pipeline(&group.middleware, config)
            .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, err)) })?;

        let middleware_chain = MiddlewareChain::new(middleware_instances);

        // Process through middleware chain
        let processed_json_envelope = middleware_chain.left(json_envelope).await?;

        tracing::info!(
            "Processing incoming middleware for {} middlewares",
            group.middleware.len()
        );

        // Convert back to Vec<u8> envelope
        let processed_envelope = RequestEnvelope {
            request_details: processed_json_envelope.request_details,
            original_data: envelope.original_data, // Keep original bytes
            normalized_data: processed_json_envelope.normalized_data,
            normalized_snapshot: processed_json_envelope.normalized_snapshot,
        };

        Ok(processed_envelope)
    }

    // Process through outgoing middleware chain
    async fn process_outgoing_middleware(
        envelope: RequestEnvelope<Vec<u8>>,
        group: &Pipeline,
        config: &Config,
    ) -> Result<RequestEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        // Clone normalized_data before using it to avoid ownership issues
        let normalized_data = envelope.normalized_data.clone();

        // Convert envelope to use serde_json::Value for middleware processing
        let json_envelope = RequestEnvelope {
            request_details: envelope.request_details.clone(),
            original_data: normalized_data.unwrap_or_else(|| {
                serde_json::from_slice(&envelope.original_data).unwrap_or(serde_json::Value::Null)
            }),
            normalized_data: envelope.normalized_data.clone(),
            normalized_snapshot: envelope.normalized_snapshot.clone(),
        };

        // Build middleware instances from pipeline names
        let middleware_instances = build_middleware_instances_for_pipeline(&group.middleware, config)
            .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, err)) })?;

        let middleware_chain = MiddlewareChain::new(middleware_instances);

        // Process through middleware chain (right side)
        let processed_json_envelope = middleware_chain.right(json_envelope).await?;

        tracing::info!(
            "Processing outgoing middleware for {} middlewares",
            group.middleware.len()
        );

        // Convert back to Vec<u8> envelope
        let processed_envelope = RequestEnvelope {
            request_details: processed_json_envelope.request_details,
            original_data: envelope.original_data, // Keep original bytes
            normalized_data: Some(processed_json_envelope.original_data),
            normalized_snapshot: processed_json_envelope.normalized_snapshot,
        };

        Ok(processed_envelope)
    }

}
