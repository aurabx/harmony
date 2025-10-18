//! # DEPRECATED: Dispatcher Module
//!
//! This module is deprecated and will be removed in Phase 6 of the protocol adapter refactoring.
//!
//! All HTTP-specific routing and request handling has been moved to:
//! - `crate::adapters::http::HttpAdapter` - HTTP protocol adapter
//! - `crate::adapters::http::router` - HTTP route building and request handling
//!
//! Pipeline execution is now handled by:
//! - `crate::pipeline::PipelineExecutor` - Protocol-agnostic pipeline processing
//!
//! This file is kept temporarily for backward compatibility during the migration.
//! Do not add new functionality here.

use crate::config::config::Config;
use crate::models::backends::backends::Backend;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::chain::MiddlewareChain;
use crate::models::middleware::middleware::build_middleware_instances_for_pipeline;
use crate::models::middleware::AuthFailure;
use crate::models::pipelines::config::Pipeline;
use axum::body::Body;
use axum::extract::Request;
use axum::{response::Response, Router};
use http::{Method, StatusCode};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[deprecated(
    since = "0.2.0",
    note = "Dispatcher is deprecated. Use HttpAdapter instead. Will be removed after Phase 6."
)]
pub struct Dispatcher {
    config: Arc<Config>,
}

impl Dispatcher {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Maps incoming middleware errors to appropriate HTTP status codes
    /// Only authentication failures (AuthFailure) should result in 401,
    /// all other errors should be 500 Internal Server Error
    fn map_incoming_middleware_error_to_status(
        err: &(dyn std::error::Error + Send + Sync + 'static),
    ) -> StatusCode {
        // Check if the error is specifically an AuthFailure
        if err.downcast_ref::<AuthFailure>().is_some() {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }

    /// Builds incoming routes for a specific group within the given app router.
    /// @todo Abstract the HTTP and DICOM parts into separate handlers
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
            .endpoint_incoming_request(
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
                    let status = Self::map_incoming_middleware_error_to_status(err.as_ref());
                    tracing::warn!("Incoming middleware failed: {:?}", err);
                    status
                })?;

        // 3. Process through backend(s) - returns ResponseEnvelope
        let response_envelope =
            Self::process_backends(after_incoming_mw, pipeline, &config, &service)
                .await
                .map_err(|err| {
                    tracing::error!("Backend processing failed: {:?}", err);
                    StatusCode::BAD_GATEWAY
                })?;

        // 4. Outgoing (right) middleware chain - operates on ResponseEnvelope
        let after_outgoing_mw =
            Self::process_outgoing_middleware(response_envelope, pipeline, &config)
                .await
                .map_err(|err| {
                    tracing::error!("Outgoing middleware failed: {:?}", err);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

        // 5. Final endpoint response processing - converts ResponseEnvelope to axum Response
        let response = service
            .endpoint_outgoing_response(
                after_outgoing_mw,
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

    // Process through backend(s) - returns ResponseEnvelope
    async fn process_backends(
        envelope: RequestEnvelope<Vec<u8>>,
        group: &Pipeline,
        config: &Config,
        service: &Box<
            dyn crate::models::services::services::ServiceType<ReqBody = serde_json::Value>,
        >,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        // Check if endpoint requested to skip backends
        let skip_backends = envelope
            .request_details
            .metadata
            .get("skip_backends")
            .map(|v| v == "true")
            .unwrap_or(false);

        if skip_backends {
            tracing::info!("Skipping backends due to endpoint 'skip_backends' flag");
            // Service must still generate a ResponseEnvelope (e.g., from endpoint's prepared response data)
            let response_envelope = service
                .backend_outgoing_request(envelope, &HashMap::new())
                .await
                .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Backend skip failed: {:?}", err),
                    ))
                })?;
            return Ok(response_envelope);
        }

        // If no backends configured, service should generate response directly
        if group.backends.is_empty() {
            tracing::info!("No backends configured - service will generate response directly");
            let response_envelope = service
                .backend_outgoing_request(envelope, &HashMap::new())
                .await
                .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Service backend_outgoing_request failed: {:?}", err),
                    ))
                })?;
            return Ok(response_envelope);
        }

        tracing::info!("Processing through {} backends", group.backends.len());

        // For simplicity, process first backend (most configs have one backend per pipeline)
        // Multi-backend chaining would require more complex envelope state management
        if let Some(backend_name) = group.backends.first() {
            if let Some(backend) = config.backends.get(backend_name) {
                return Self::process_single_backend(envelope, backend).await;
            } else {
                tracing::warn!("Backend '{}' not found in config", backend_name);
            }
        }

        // Backend referenced but not found - return a 502 response
        Ok(ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            502,
            HashMap::from([("content-type".to_string(), "text/plain".to_string())]),
            b"Backend not found in configuration".to_vec(),
            None,
        ))
    }

    // Process through a single backend - returns ResponseEnvelope
    async fn process_single_backend(
        envelope: RequestEnvelope<Vec<u8>>,
        backend: &Backend,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let service = backend
            .resolve_service()
            .map_err(|err| format!("Failed to resolve backend service: {}", err))?;

        // Call backend service - now returns ResponseEnvelope
        let response = service
            .backend_outgoing_request(
                envelope,
                backend.options.as_ref().unwrap_or(&HashMap::new()),
            )
            .await
            .map_err(|err| format!("Backend request failed: {:?}", err))?;

        tracing::info!(
            "Processed backend '{}' using service type '{}'",
            backend.service,
            backend.service
        );

        Ok(response)
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
            backend_request_details: envelope.backend_request_details.clone(),
            original_data: normalized_data.unwrap_or_else(|| {
                serde_json::from_slice(&envelope.original_data).unwrap_or(serde_json::Value::Null)
            }),
            normalized_data: envelope.normalized_data.clone(),
            normalized_snapshot: envelope.normalized_snapshot.clone(),
        };

        // Build middleware instances from pipeline names
        let middleware_instances =
            build_middleware_instances_for_pipeline(&group.middleware, config).map_err(
                |err| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
                },
            )?;

        let middleware_chain = MiddlewareChain::new(middleware_instances);

        // Process through middleware chain
        let processed_json_envelope = middleware_chain.left(json_envelope).await?;

        tracing::info!(
            "Processing incoming middleware for {} middlewares",
            group.middleware.len()
        );

        // Convert back to Vec<u8> envelope
        // @todo No idea why we are converting everything to json then back to Vec<u8>
        let processed_envelope = RequestEnvelope {
            request_details: processed_json_envelope.request_details,
            backend_request_details: processed_json_envelope.backend_request_details,
            original_data: envelope.original_data, // Keep original bytes
            normalized_data: processed_json_envelope.normalized_data,
            normalized_snapshot: processed_json_envelope.normalized_snapshot,
        };

        Ok(processed_envelope)
    }

    // Process through outgoing middleware chain - operates on ResponseEnvelope
    async fn process_outgoing_middleware(
        envelope: ResponseEnvelope<Vec<u8>>,
        group: &Pipeline,
        config: &Config,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(
            "Processing outgoing middleware for {} middlewares",
            group.middleware.len()
        );

        // Convert ResponseEnvelope<Vec<u8>> to ResponseEnvelope<serde_json::Value> for middleware
        let json_envelope =
            envelope
                .to_json()
                .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Failed to convert response to JSON: {}", err),
                    ))
                })?;

        // Build middleware instances from pipeline names
        let middleware_instances =
            build_middleware_instances_for_pipeline(&group.middleware, config).map_err(
                |err| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
                },
            )?;

        let middleware_chain = MiddlewareChain::new(middleware_instances);

        // Process through middleware chain (right side) - now works with ResponseEnvelope
        let processed_json_envelope = middleware_chain.right(json_envelope).await?;

        // Convert back to ResponseEnvelope<Vec<u8>>
        let processed_envelope = processed_json_envelope.to_bytes().map_err(
            |err| -> Box<dyn std::error::Error + Send + Sync> {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Failed to convert response to bytes: {}", err),
                ))
            },
        )?;

        Ok(processed_envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::middleware::AuthFailure;

    #[test]
    fn test_map_incoming_middleware_error_to_status_authfailure() {
        let auth_err = AuthFailure("test");
        let status = Dispatcher::map_incoming_middleware_error_to_status(&auth_err);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_map_incoming_middleware_error_to_status_authfailure_missing_creds() {
        let auth_err = AuthFailure("Missing Authorization header");
        let status = Dispatcher::map_incoming_middleware_error_to_status(&auth_err);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_map_incoming_middleware_error_to_status_authfailure_expired() {
        let auth_err = AuthFailure("jwt verify failed");
        let status = Dispatcher::map_incoming_middleware_error_to_status(&auth_err);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_map_incoming_middleware_error_to_status_non_auth_error() {
        let generic_err = std::io::Error::new(std::io::ErrorKind::Other, "boom");
        let status = Dispatcher::map_incoming_middleware_error_to_status(&generic_err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_map_incoming_middleware_error_to_status_string_error() {
        #[derive(Debug)]
        struct StringError(String);
        impl std::fmt::Display for StringError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
        impl std::error::Error for StringError {}

        let string_err = StringError("generic error".to_string());
        let status = Dispatcher::map_incoming_middleware_error_to_status(&string_err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
