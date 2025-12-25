use crate::config::config::Config;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope, TargetDetails};
use crate::models::mesh::config::Mesh;
use crate::models::middleware::chain::MiddlewareChain;
use crate::models::middleware::middleware::{build_middleware_instances_for_pipeline, Middleware};
use crate::models::middleware::types::mesh_auth::MeshAuthMiddleware;
use crate::models::pipelines::config::Pipeline;
use crate::models::protocol::ProtocolCtx;
use std::collections::HashMap;

/// Error type for pipeline execution
#[derive(Debug)]
pub enum PipelineError {
    ServiceError(String),
    MiddlewareError(Box<dyn std::error::Error + Send + Sync>),
    BackendError(String),
    ConfigError(String),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::ServiceError(msg) => write!(f, "Service error: {}", msg),
            PipelineError::MiddlewareError(err) => write!(f, "Middleware error: {}", err),
            PipelineError::BackendError(msg) => write!(f, "Backend error: {}", msg),
            PipelineError::ConfigError(msg) => write!(f, "Config error: {}", msg),
        }
    }
}

impl std::error::Error for PipelineError {}

impl From<String> for PipelineError {
    fn from(msg: String) -> Self {
        PipelineError::ServiceError(msg)
    }
}

impl From<&str> for PipelineError {
    fn from(msg: &str) -> Self {
        PipelineError::ServiceError(msg.to_string())
    }
}

/// Protocol-agnostic pipeline executor
///
/// This is the single source of truth for all request processing,
/// regardless of protocol (HTTP, DIMSE, HL7, etc.)
pub struct PipelineExecutor;

impl PipelineExecutor {
    /// Execute a request through the complete pipeline
    ///
    /// # Flow
    /// 1. Endpoint service preprocessing
    /// 2. Resolve target_details from backend config
    /// 3. Incoming middleware chain (left)
    /// 4. Backend invocation
    /// 5. Outgoing middleware chain (right)
    /// 6. Endpoint service post-processing (protocol-aware)
    /// 7. Return ResponseEnvelope
    ///
    /// # Arguments
    /// * `envelope` - The request envelope to process
    /// * `pipeline_name` - The pipeline key/name in the config
    /// * `pipeline` - Pipeline configuration (endpoints, backends, middleware)
    /// * `config` - Full application configuration
    /// * `ctx` - Protocol context for protocol-specific metadata
    ///
    /// # Returns
    /// ResponseEnvelope on success, PipelineError on failure
    #[tracing::instrument(skip(envelope, pipeline_name, pipeline, config, ctx), fields(
        protocol = ?ctx.protocol,
        pipeline = pipeline_name
    ))]
    pub async fn execute(
        envelope: RequestEnvelope<Vec<u8>>,
        pipeline_name: &str,
        pipeline: &Pipeline,
        config: &Config,
        ctx: &ProtocolCtx,
    ) -> Result<ResponseEnvelope<Vec<u8>>, PipelineError> {
        tracing::info!("Executing pipeline '{}' for protocol: {:?}", pipeline_name, ctx.protocol);

        // 1. Endpoint service preprocessing
        let envelope = Self::process_endpoint_incoming(envelope, pipeline, config).await?;

        // 2. Resolve target_details from backend config (so middleware has access)
        let envelope = Self::resolve_target_details(envelope, pipeline, config).await?;

        // 3. Incoming middleware chain (left)
        let envelope = Self::process_incoming_middleware(envelope, pipeline_name, pipeline, config).await?;

        // 4. Backend invocation
        let response = Self::process_backends(envelope, pipeline, config).await?;

        // 5. Outgoing middleware chain (right)
        let mut response = Self::process_outgoing_middleware(response, pipeline, config).await?;

        // 6. Endpoint service post-processing (protocol-aware)
        Self::process_endpoint_outgoing(&mut response, pipeline, config, ctx).await?;

        tracing::info!("Pipeline execution completed successfully");
        Ok(response)
    }

    /// Resolve target_details from backend configuration
    ///
    /// Creates TargetDetails by:
    /// 1. Starting with request_details as the base (method, uri, headers, etc.)
    /// 2. Overlaying any target configuration from the backend (base_url, base_path, etc.)
    ///
    /// This ensures middleware always has complete target info to work with.
    async fn resolve_target_details(
        mut envelope: RequestEnvelope<Vec<u8>>,
        pipeline: &Pipeline,
        config: &Config,
    ) -> Result<RequestEnvelope<Vec<u8>>, PipelineError> {
        // Skip if target_details already set (e.g., by endpoint)
        if envelope.target_details.is_some() {
            return Ok(envelope);
        }

        // Start with request_details as the base
        let path = crate::models::services::path_utils::extract_path(&envelope);
        let mut target = TargetDetails {
            base_url: String::new(),
            method: envelope.request_details.method.clone(),
            uri: path,
            headers: envelope.request_details.headers.clone(),
            cookies: envelope.request_details.cookies.clone(),
            query_params: envelope.request_details.query_params.clone(),
            metadata: envelope.request_details.metadata.clone(),
        };

        // Get first backend from pipeline to overlay target config
        let backend_name = match pipeline.backends.first() {
            Some(name) => name,
            None => {
                // No backends - use request-based target_details
                envelope.target_details = Some(target);
                return Ok(envelope);
            }
        };

        let backend = match config.backends.get(backend_name) {
            Some(b) => b,
            None => {
                // Backend not found - use request-based target_details
                envelope.target_details = Some(target);
                return Ok(envelope);
            }
        };

        // Overlay backend's connection config (resolved from target_ref if present)
        if let Some(conn) = &backend.connection {
            target.base_url = conn.to_base_url();

            // Add protocol to metadata if available
            if let Some(protocol) = &conn.protocol {
                target.metadata.insert("protocol".to_string(), protocol.clone());
            }
        } else if let Some(base_url) = backend
            .options
            .as_ref()
            .and_then(|opts| opts.get("base_url"))
            .and_then(|v| v.as_str())
        {
            // Fallback to base_url in options (legacy configuration)
            target.base_url = base_url.to_string();
        }

        // Add reliability settings to metadata
        if let Some(timeout) = backend.timeout_secs {
            target.metadata.insert("timeout_secs".to_string(), timeout.to_string());
        }
        if let Some(retries) = backend.max_retries {
            target.metadata.insert("max_retries".to_string(), retries.to_string());
        }

        tracing::debug!(
            "Resolved target_details from backend '{}': base_url='{}', method='{}', uri='{}'",
            backend_name,
            target.base_url,
            target.method,
            target.uri
        );

        envelope.target_details = Some(target);
        Ok(envelope)
    }

    /// Process endpoint incoming request
    async fn process_endpoint_incoming(
        envelope: RequestEnvelope<Vec<u8>>,
        pipeline: &Pipeline,
        config: &Config,
    ) -> Result<RequestEnvelope<Vec<u8>>, PipelineError> {
        // Get first endpoint from pipeline
        let endpoint_name = pipeline
            .endpoints
            .first()
            .ok_or_else(|| PipelineError::ConfigError("No endpoints in pipeline".to_string()))?;

        let endpoint = config.endpoints.get(endpoint_name).ok_or_else(|| {
            PipelineError::ConfigError(format!("Endpoint '{}' not found", endpoint_name))
        })?;

        let service = endpoint.resolve_service().map_err(|e| {
            PipelineError::ServiceError(format!("Failed to resolve service: {}", e))
        })?;

        let empty_options = HashMap::new();
        let options = endpoint.options.as_ref().unwrap_or(&empty_options);

        service
            .endpoint_incoming_request(envelope, options)
            .await
            .map_err(|e| PipelineError::ServiceError(format!("Endpoint incoming failed: {}", e)))
    }

    /// Process incoming middleware chain
    ///
    /// Automatically injects MeshAuth middleware when in a mesh context:
    /// - For ingress: MeshAuth validation is prepended (first middleware)
    /// - For egress: MeshAuth JWT generation is appended (last middleware, before backend)
    async fn process_incoming_middleware(
        envelope: RequestEnvelope<Vec<u8>>,
        pipeline_name: &str,
        pipeline: &Pipeline,
        config: &Config,
    ) -> Result<RequestEnvelope<Vec<u8>>, PipelineError> {
        let left_chain = pipeline.middleware.left_chain();
        tracing::debug!(
            "Pipeline '{}' middleware config: {:?}",
            pipeline.description,
            pipeline.middleware
        );
        tracing::debug!(
            "Processing incoming middleware for {} middlewares: {:?}",
            left_chain.len(),
            left_chain
        );

        // Convert to JSON envelope for middleware processing
        let normalized_data = envelope.normalized_data.clone();
        let json_value = normalized_data.unwrap_or_else(|| {
            serde_json::from_slice(&envelope.original_data).unwrap_or(serde_json::Value::Null)
        });

        let json_envelope = RequestEnvelope::builder()
            .method(envelope.request_details.method.clone())
            .uri(envelope.request_details.uri.clone())
            .headers(envelope.request_details.headers.clone())
            .cookies(envelope.request_details.cookies.clone())
            .query_params(envelope.request_details.query_params.clone())
            .cache_status(envelope.request_details.cache_status.clone())
            .metadata(envelope.request_details.metadata.clone())
            .backend_request_details(envelope.backend_request_details.clone())
            .target_details(envelope.target_details.clone())
            .original_data(json_value.clone())
            .normalized_data(Some(json_value))
            .normalized_snapshot(envelope.normalized_snapshot.clone())
            .build()
            .expect("Failed to build json_envelope");

        // Build middleware instances from pipeline config
        let mut middleware_instances: Vec<Box<dyn Middleware>> =
            build_middleware_instances_for_pipeline(&left_chain, config).map_err(
                |err| {
                    PipelineError::MiddlewareError(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        err,
                    )))
                },
            )?;

        // Auto-inject MeshAuth middleware based on mesh context
        Self::inject_mesh_auth_middleware(
            &mut middleware_instances,
            &envelope,
            pipeline_name,
            pipeline,
            config,
        )?;

        let middleware_chain = MiddlewareChain::new(middleware_instances);

        // Process through middleware chain
        let processed_json_envelope = middleware_chain
            .left(json_envelope)
            .await
            .map_err(PipelineError::MiddlewareError)?;

        // Convert back to Vec<u8> envelope
        let processed_envelope = RequestEnvelope::builder()
            .method(processed_json_envelope.request_details.method)
            .uri(processed_json_envelope.request_details.uri)
            .headers(processed_json_envelope.request_details.headers)
            .cookies(processed_json_envelope.request_details.cookies)
            .query_params(processed_json_envelope.request_details.query_params)
            .cache_status(processed_json_envelope.request_details.cache_status)
            .metadata(processed_json_envelope.request_details.metadata)
            .backend_request_details(processed_json_envelope.backend_request_details)
            .target_details(processed_json_envelope.target_details)
            .original_data(envelope.original_data)
            .normalized_data(processed_json_envelope.normalized_data)
            .normalized_snapshot(processed_json_envelope.normalized_snapshot)
            .build()
            .expect("Failed to build processed_envelope");

        Ok(processed_envelope)
    }

    /// Inject MeshAuth middleware based on mesh context
    ///
    /// - Ingress: If request came through mesh routing (has mesh_name in metadata),
    ///   prepend MeshAuth validation middleware
    /// - Egress: If pipeline's backend is part of a mesh egress,
    ///   append MeshAuth JWT generation middleware
    fn inject_mesh_auth_middleware(
        middleware_instances: &mut Vec<Box<dyn Middleware>>,
        envelope: &RequestEnvelope<Vec<u8>>,
        pipeline_name: &str,
        pipeline: &Pipeline,
        config: &Config,
    ) -> Result<(), PipelineError> {
        // Check for ingress context (request came from another mesh member)
        if let Some(mesh_name) = envelope.request_details.metadata.get("mesh_name") {
            if let Some(mesh) = config.mesh.get(mesh_name) {
                if mesh.enabled {
                    tracing::debug!(
                        "Injecting MeshAuth ingress middleware for mesh '{}'",
                        mesh_name
                    );
                    
                    let ingress_mw = Self::create_mesh_auth_ingress(mesh_name, mesh)?;
                    // Prepend for ingress - validate JWT first
                    middleware_instances.insert(0, Box::new(ingress_mw));
                }
            }
        }

        // Check for egress context: if destination URL matches an ingress in a mesh
        // that also contains the current pipeline's egress, inject auth
        tracing::debug!(
            "Checking egress context for pipeline '{}' with backends: {:?}",
            pipeline_name,
            pipeline.backends
        );
        if let Some(backend_name) = pipeline.backends.first() {
            if let Some(backend) = config.backends.get(backend_name) {
                // Get destination URL from backend connection or options.base_url
                let destination_url = backend
                    .connection
                    .as_ref()
                    .map(|c| c.to_base_url())
                    .or_else(|| {
                        backend
                            .options
                            .as_ref()
                            .and_then(|opts| opts.get("base_url"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    });
                
                tracing::debug!(
                    "Backend '{}' destination URL: {:?}",
                    backend_name,
                    destination_url
                );

                if let Some(ref dest_url) = destination_url {
                    let mesh_match = Self::find_mesh_for_egress(
                        pipeline_name,
                        pipeline,
                        dest_url,
                        config,
                    );

                    // Check if any egress for this pipeline has mode=mesh
                    let egress_requires_mesh = Self::pipeline_egress_requires_mesh(
                        pipeline_name,
                        config,
                    );

                    if let Some((mesh_name, mesh)) = mesh_match {
                        if mesh.enabled {
                            tracing::debug!(
                                "Injecting MeshAuth egress middleware for mesh '{}' (destination '{}')",
                                mesh_name,
                                dest_url
                            );
                            
                            let egress_mw = Self::create_mesh_auth_egress(&mesh_name, mesh)?;
                            // Append for egress - generate JWT last (before backend)
                            middleware_instances.push(Box::new(egress_mw));
                        }
                    } else if egress_requires_mesh {
                        // Egress mode=mesh but no mesh matched - reject the request
                        tracing::warn!(
                            "Egress rejected: pipeline '{}' egress requires mesh auth but destination '{}' doesn't match any mesh",
                            pipeline.description,
                            dest_url
                        );
                        return Err(PipelineError::ConfigError(
                            "Egress requires mesh authentication but destination does not match any mesh".to_string()
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// Check if any egress for this pipeline has mode=mesh
    fn pipeline_egress_requires_mesh(pipeline_name: &str, config: &Config) -> bool {
        use crate::models::mesh::config::IngressEgressMode;

        for egress in config.egress.values() {
            if egress.pipeline == pipeline_name 
                && egress.enabled 
                && egress.mode == IngressEgressMode::Mesh 
            {
                return true;
            }
        }
        false
    }

    /// Find mesh configuration for an egress based on destination URL matching.
    ///
    /// For auth to be added, BOTH conditions must be met:
    /// a) The current pipeline has an egress defined in the mesh
    /// b) The destination URL matches an ingress/remote_ingress URL pattern in that same mesh
    fn find_mesh_for_egress<'a>(
        pipeline_name: &str,
        pipeline: &Pipeline,
        destination_url: &str,
        config: &'a Config,
    ) -> Option<(String, &'a Mesh)> {
        // Parse destination URL for matching
        let dest_parsed = url::Url::parse(destination_url).ok()?;
        let dest_scheme = dest_parsed.scheme();
        let dest_host = dest_parsed.host_str()?;
        let dest_port = dest_parsed.port();
        let dest_path = dest_parsed.path();

        // Get the first backend name from pipeline for fallback matching
        let first_backend = pipeline.backends.first();

        tracing::debug!(
            "find_mesh_for_egress: checking {} meshes, {} egresses, {} remote_ingresses",
            config.mesh.len(),
            config.egress.len(),
            config.remote_ingress.len()
        );

        // Check each mesh
        for (mesh_name, mesh) in &config.mesh {
            tracing::debug!(
                "Checking mesh '{}': enabled={}, ingress={:?}, egress={:?}",
                mesh_name,
                mesh.enabled,
                mesh.ingress,
                mesh.egress
            );
            if !mesh.enabled {
                continue;
            }

            // a) Check if this mesh has an egress for the current pipeline
            let has_pipeline_egress = mesh.egress.iter().any(|egress_name| {
                let egress_opt = config.egress.get(egress_name);
                tracing::debug!(
                    "  Checking egress '{}': found={}, value={:?}",
                    egress_name,
                    egress_opt.is_some(),
                    egress_opt.map(|e| (&e.pipeline, e.enabled, &e.backend))
                );
                egress_opt.map_or(false, |egress| {
                    // Must match pipeline name first
                    if egress.pipeline != pipeline_name || !egress.enabled {
                        tracing::debug!(
                            "    Egress '{}' pipeline mismatch: egress.pipeline='{}' != pipeline_name='{}' or disabled",
                            egress_name,
                            egress.pipeline,
                            pipeline_name
                        );
                        return false;
                    }
                    // Then check backend: if specified must match, otherwise first backend
                    if let Some(ref egress_backend) = egress.backend {
                        let matches = pipeline.backends.contains(egress_backend);
                        tracing::debug!(
                            "    Egress '{}' backend check: egress_backend='{}' in pipeline.backends={:?} = {}",
                            egress_name,
                            egress_backend,
                            pipeline.backends,
                            matches
                        );
                        matches
                    } else {
                        // No backend specified - matches if pipeline has any backends
                        let matches = first_backend.is_some();
                        tracing::debug!(
                            "    Egress '{}' has no backend specified, pipeline has backends: {}",
                            egress_name,
                            matches
                        );
                        matches
                    }
                })
            });

            if !has_pipeline_egress {
                tracing::debug!("  Mesh '{}' has no matching pipeline egress, skipping", mesh_name);
                continue;
            }

            // b) Check if destination URL matches any ingress in this mesh
            for ingress_name in &mesh.ingress {
                // First check local ingress definitions
                if let Some(ingress) = config.ingress.get(ingress_name) {
                    if !ingress.enabled {
                        continue;
                    }

                    for url_str in &ingress.urls {
                        if Self::url_matches_pattern(dest_scheme, dest_host, dest_port, dest_path, url_str) {
                            tracing::debug!(
                                "Egress destination '{}' matches local ingress '{}' in mesh '{}'",
                                destination_url,
                                ingress_name,
                                mesh_name
                            );
                            return Some((mesh_name.clone(), mesh));
                        }
                    }
                }

                // Then check remote ingress definitions
                if let Some(remote_ingress) = config.remote_ingress.get(ingress_name) {
                    for url_str in &remote_ingress.urls {
                        if Self::url_matches_pattern(dest_scheme, dest_host, dest_port, dest_path, url_str) {
                            tracing::debug!(
                                "Egress destination '{}' matches remote ingress '{}' in mesh '{}'",
                                destination_url,
                                ingress_name,
                                mesh_name
                            );
                            return Some((mesh_name.clone(), mesh));
                        }
                    }
                }
            }
        }

        None
    }

    /// Check if a URL matches an ingress URL pattern.
    fn url_matches_pattern(
        scheme: &str,
        host: &str,
        port: Option<u16>,
        path: &str,
        pattern_url: &str,
    ) -> bool {
        let pattern = match url::Url::parse(pattern_url) {
            Ok(p) => p,
            Err(_) => return false,
        };

        // Scheme must match
        if pattern.scheme() != scheme {
            return false;
        }

        // Host must match
        if pattern.host_str() != Some(host) {
            return false;
        }

        // Port must match if specified in pattern
        if let Some(pattern_port) = pattern.port() {
            if port != Some(pattern_port) {
                return false;
            }
        }

        // Path must start with pattern's path (prefix match)
        path.starts_with(pattern.path())
    }

    /// Create MeshAuth middleware for ingress (JWT validation)
    fn create_mesh_auth_ingress(
        mesh_name: &str,
        mesh: &Mesh,
    ) -> Result<MeshAuthMiddleware, PipelineError> {
        MeshAuthMiddleware::for_ingress(
            mesh_name.to_string(),
            mesh.provider.clone(),
            mesh.jwt_secret.clone(),
            mesh.jwt_public_key_path.clone(),
        )
        .map_err(|e| {
            PipelineError::ConfigError(format!(
                "Failed to create MeshAuth ingress middleware for mesh '{}': {}",
                mesh_name, e
            ))
        })
    }

    /// Create MeshAuth middleware for egress (JWT generation)
    fn create_mesh_auth_egress(
        mesh_name: &str,
        mesh: &Mesh,
    ) -> Result<MeshAuthMiddleware, PipelineError> {
        MeshAuthMiddleware::for_egress(
            mesh_name.to_string(),
            mesh.provider.clone(),
            mesh.jwt_secret.clone(),
            mesh.jwt_private_key_path.clone(),
        )
        .map_err(|e| {
            PipelineError::ConfigError(format!(
                "Failed to create MeshAuth egress middleware for mesh '{}': {}",
                mesh_name, e
            ))
        })
    }

    /// Process through backends
    async fn process_backends(
        envelope: RequestEnvelope<Vec<u8>>,
        pipeline: &Pipeline,
        config: &Config,
    ) -> Result<ResponseEnvelope<Vec<u8>>, PipelineError> {
        tracing::debug!("Processing through {} backends", pipeline.backends.len());

        // Check if endpoint requested to skip backends
        let skip_backends = envelope
            .request_details
            .metadata
            .get("skip_backends")
            .map(|v| v == "true")
            .unwrap_or(false);

        if skip_backends {
            tracing::info!("Skipping backends - building response from request envelope");
            // When backends are skipped, build response directly from the request envelope
            // Middleware/endpoint should have prepared normalized_data with the response

            // Try to extract status code from normalized_data.response.status
            let (status_code, body) = if let Some(ref normalized) = envelope.normalized_data {
                let status = normalized
                    .get("response")
                    .and_then(|r| r.get("status"))
                    .and_then(|s| s.as_u64())
                    .map(|s| s as u16)
                    .unwrap_or(200);

                // Try to extract body from normalized_data.response.body
                let body = normalized
                    .get("response")
                    .and_then(|r| r.get("body"))
                    .map(|b| {
                        if b.is_string() {
                            b.as_str().unwrap_or("").as_bytes().to_vec()
                        } else {
                            serde_json::to_vec(b).unwrap_or_default()
                        }
                    })
                    .unwrap_or_else(|| serde_json::to_vec(normalized).unwrap_or_default());

                (status, body)
            } else {
                (200, Vec::new())
            };

            let mut response = ResponseEnvelope::from_backend(
                envelope.request_details.clone(),
                status_code,
                HashMap::new(),
                body,
                None,
            );

            // Preserve normalized_data for outgoing middleware
            response.normalized_data = envelope.normalized_data;

            return Ok(response);
        }

        // If no backends configured, return empty response
        if pipeline.backends.is_empty() {
            tracing::info!("No backends configured - returning empty response");
            return Ok(ResponseEnvelope::from_backend(
                envelope.request_details.clone(),
                200,
                HashMap::new(),
                Vec::new(),
                None,
            ));
        }

        // Process first backend (most configs have one backend per pipeline)
        if let Some(backend_name) = pipeline.backends.first() {
            if let Some(backend) = config.backends.get(backend_name) {
                let service = backend.resolve_service().map_err(|e| {
                    PipelineError::BackendError(format!("Failed to resolve backend service: {}", e))
                })?;

                let response = service
                    .backend_outgoing_request(
                        envelope,
                        backend.options.as_ref().unwrap_or(&HashMap::new()),
                    )
                    .await
                    .map_err(|e| {
                        PipelineError::BackendError(format!("Backend request failed: {:?}", e))
                    })?;

                return Ok(response);
            } else {
                tracing::warn!("Backend '{}' not found in config", backend_name);
            }
        }

        // Backend referenced but not found - return 502
        Ok(ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            502,
            HashMap::from([("content-type".to_string(), "text/plain".to_string())]),
            b"Backend not found in configuration".to_vec(),
            None,
        ))
    }

    /// Process outgoing middleware chain
    async fn process_outgoing_middleware(
        envelope: ResponseEnvelope<Vec<u8>>,
        pipeline: &Pipeline,
        config: &Config,
    ) -> Result<ResponseEnvelope<Vec<u8>>, PipelineError> {
        let right_chain = pipeline.middleware.right_chain();
        tracing::debug!(
            "Processing outgoing middleware for {} middlewares",
            right_chain.len()
        );

        // Convert to JSON envelope for middleware processing
        let json_envelope = envelope.to_json().map_err(|e| {
            PipelineError::MiddlewareError(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Failed to convert response to JSON: {}", e),
            )))
        })?;

        // Build middleware instances
        let middleware_instances =
            build_middleware_instances_for_pipeline(&right_chain, config).map_err(
                |err| {
                    PipelineError::MiddlewareError(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        err,
                    )))
                },
            )?;

        let middleware_chain = MiddlewareChain::new(middleware_instances);

        // Determine if we should reverse the chain
        // For List format: reverse to mirror left chain
        // For Split format: use exact order specified by user
        let should_reverse = pipeline.middleware.should_reverse_right();

        // Process through middleware chain (right side)
        let processed_json_envelope = middleware_chain
            .right(json_envelope, should_reverse)
            .await
            .map_err(PipelineError::MiddlewareError)?;

        // Convert back to Vec<u8> envelope
        let processed_envelope = processed_json_envelope.to_bytes().map_err(|e| {
            PipelineError::MiddlewareError(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Failed to convert response to bytes: {}", e),
            )))
        })?;

        Ok(processed_envelope)
    }

    /// Process endpoint outgoing response (protocol-aware)
    async fn process_endpoint_outgoing(
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        pipeline: &Pipeline,
        config: &Config,
        ctx: &ProtocolCtx,
    ) -> Result<(), PipelineError> {
        tracing::debug!("Processing endpoint outgoing response");

        // Get first endpoint from pipeline
        let endpoint_name = pipeline
            .endpoints
            .first()
            .ok_or_else(|| PipelineError::ConfigError("No endpoints in pipeline".to_string()))?;

        let endpoint = config.endpoints.get(endpoint_name).ok_or_else(|| {
            PipelineError::ConfigError(format!("Endpoint '{}' not found", endpoint_name))
        })?;

        let service = endpoint.resolve_service().map_err(|e| {
            PipelineError::ServiceError(format!("Failed to resolve service: {}", e))
        })?;

        let empty_options = HashMap::new();
        let options = endpoint.options.as_ref().unwrap_or(&empty_options);

        // Call protocol-aware endpoint outgoing hook
        service
            .endpoint_outgoing_protocol(envelope, ctx, options)
            .await
            .map_err(|e| PipelineError::ServiceError(format!("Endpoint outgoing failed: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::backends::backends::Backend;
    use crate::models::connection::ConnectionConfig;
    use crate::models::envelope::envelope::RequestEnvelope;
    use crate::models::pipelines::config::PipelineMiddleware;

    #[test]
    fn test_pipeline_error_display() {
        let err = PipelineError::ServiceError("test error".to_string());
        assert_eq!(err.to_string(), "Service error: test error");

        let err = PipelineError::BackendError("backend failed".to_string());
        assert_eq!(err.to_string(), "Backend error: backend failed");
    }

    #[test]
    fn test_pipeline_error_from_string() {
        let err: PipelineError = "test".into();
        assert_eq!(err.to_string(), "Service error: test");
    }

    #[tokio::test]
    async fn test_resolve_target_details_from_backend_connection() {
        let mut config = Config::default();

        // Add a backend with connection config (simulates resolved target_ref)
        config.backends.insert(
            "test_backend".to_string(),
            Backend {
                service: "http".to_string(),
                target_ref: None,
                connection: Some(ConnectionConfig {
                    host: "api.example.com".to_string(),
                    port: Some(443),
                    protocol: Some("https".to_string()),
                    base_path: Some("/v1".to_string()),
                    ca_cert_path: None,
                }),
                authentication: None,
                timeout_secs: Some(60),
                max_retries: Some(3),
                options: None,
            },
        );

        let pipeline = Pipeline {
            description: "test pipeline".to_string(),
            networks: vec![],
            endpoints: vec![],
            backends: vec!["test_backend".to_string()],
            middleware: PipelineMiddleware::default(),
            ..Default::default()
        };

        // Create envelope with request details
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/users?id=123")
            .query_params(HashMap::from([(
                "id".to_string(),
                vec!["123".to_string()],
            )]))
            .header("x-custom", "value")
            .original_data(vec![])
            .build()
            .unwrap();

        // Should have no target_details initially
        assert!(envelope.target_details.is_none());

        // Resolve target details
        let result = PipelineExecutor::resolve_target_details(envelope, &pipeline, &config).await;
        assert!(result.is_ok());

        let envelope = result.unwrap();
        assert!(envelope.target_details.is_some());

        // TargetDetails starts from request_details, with backend config overlaid
        let target = envelope.target_details.unwrap();
        // base_url comes from backend connection
        assert_eq!(target.base_url, "https://api.example.com:443/v1");
        // method, uri, headers, query_params come from request_details
        assert_eq!(target.method, "GET");
        assert_eq!(target.uri, "/users"); // Path without query string
        assert_eq!(
            target.query_params.get("id"),
            Some(&vec!["123".to_string()])
        );
        assert_eq!(target.headers.get("x-custom"), Some(&"value".to_string()));
        // Metadata includes protocol and reliability settings from target
        assert_eq!(target.metadata.get("protocol"), Some(&"https".to_string()));
        assert_eq!(target.metadata.get("timeout_secs"), Some(&"60".to_string()));
        assert_eq!(target.metadata.get("max_retries"), Some(&"3".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_target_details_no_backend() {
        let config = Config::default();

        let pipeline = Pipeline {
            description: "test pipeline".to_string(),
            networks: vec![],
            endpoints: vec![],
            backends: vec![], // No backends
            middleware: PipelineMiddleware::default(),
            ..Default::default()
        };

        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .original_data(vec![])
            .build()
            .unwrap();

        let result = PipelineExecutor::resolve_target_details(envelope, &pipeline, &config).await;
        assert!(result.is_ok());

        // Should return envelope with target_details built from request_details
        let envelope = result.unwrap();
        assert!(envelope.target_details.is_some());
        let target = envelope.target_details.unwrap();
        assert_eq!(target.base_url, ""); // No backend, so no base_url
        assert_eq!(target.method, "GET"); // From request_details
        assert_eq!(target.uri, "/test"); // From request_details
    }

    #[tokio::test]
    async fn test_resolve_target_details_preserves_existing() {
        let config = Config::default();

        let pipeline = Pipeline {
            description: "test pipeline".to_string(),
            networks: vec![],
            endpoints: vec![],
            backends: vec![],
            middleware: PipelineMiddleware::default(),
            ..Default::default()
        };

        // Create envelope with existing target_details
        let existing_target = TargetDetails {
            base_url: "https://existing.com".to_string(),
            method: "POST".to_string(),
            uri: "/existing".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            metadata: HashMap::new(),
        };

        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .target_details(Some(existing_target.clone()))
            .original_data(vec![])
            .build()
            .unwrap();

        let result = PipelineExecutor::resolve_target_details(envelope, &pipeline, &config).await;
        assert!(result.is_ok());

        let envelope = result.unwrap();
        let target = envelope.target_details.unwrap();

        // Should preserve existing target_details
        assert_eq!(target.base_url, "https://existing.com");
        assert_eq!(target.method, "POST");
        assert_eq!(target.uri, "/existing");
    }

    #[test]
    fn test_url_matches_pattern_exact() {
        // Exact match
        assert!(PipelineExecutor::url_matches_pattern(
            "https",
            "api.example.com",
            None,
            "/v1/users",
            "https://api.example.com/v1"
        ));
    }

    #[test]
    fn test_url_matches_pattern_path_prefix() {
        // Path prefix match
        assert!(PipelineExecutor::url_matches_pattern(
            "https",
            "api.example.com",
            None,
            "/v1/users/123",
            "https://api.example.com/v1"
        ));
    }

    #[test]
    fn test_url_matches_pattern_wrong_scheme() {
        // Wrong scheme should not match
        assert!(!PipelineExecutor::url_matches_pattern(
            "http",
            "api.example.com",
            None,
            "/v1/users",
            "https://api.example.com/v1"
        ));
    }

    #[test]
    fn test_url_matches_pattern_wrong_host() {
        // Wrong host should not match
        assert!(!PipelineExecutor::url_matches_pattern(
            "https",
            "other.example.com",
            None,
            "/v1/users",
            "https://api.example.com/v1"
        ));
    }

    #[test]
    fn test_url_matches_pattern_wrong_path() {
        // Path doesn't start with pattern
        assert!(!PipelineExecutor::url_matches_pattern(
            "https",
            "api.example.com",
            None,
            "/v2/users",
            "https://api.example.com/v1"
        ));
    }

    #[test]
    fn test_url_matches_pattern_with_port() {
        // Port must match when specified
        assert!(PipelineExecutor::url_matches_pattern(
            "https",
            "api.example.com",
            Some(8443),
            "/v1/users",
            "https://api.example.com:8443/v1"
        ));

        // Wrong port should not match
        assert!(!PipelineExecutor::url_matches_pattern(
            "https",
            "api.example.com",
            Some(443),
            "/v1/users",
            "https://api.example.com:8443/v1"
        ));
    }

    #[test]
    fn test_find_mesh_for_egress_matching() {
        use crate::models::mesh::config::{Mesh, MeshEgress, MeshIngress, MeshProtocol, MeshProvider};

        let mut config = Config::default();

        // Add ingress with URL pattern
        config.ingress.insert(
            "partner-ingress".to_string(),
            MeshIngress {
                pipeline: "partner-pipeline".to_string(),
                ingress_type: MeshProtocol::Http,
                urls: vec!["https://partner.example.com/api".to_string()],
                enabled: true,
                ..Default::default()
            },
        );

        // Add egress for our pipeline
        config.egress.insert(
            "my-egress".to_string(),
            MeshEgress {
                pipeline: "my-pipeline".to_string(),
                egress_type: MeshProtocol::Http,
                enabled: true,
                ..Default::default()
            },
        );

        // Add mesh that includes both
        config.mesh.insert(
            "partner-mesh".to_string(),
            Mesh {
                mesh_type: MeshProtocol::Http,
                provider: MeshProvider::Local,
                jwt_secret: Some("test-secret".to_string()),
                ingress: vec!["partner-ingress".to_string()],
                egress: vec!["my-egress".to_string()],
                enabled: true,
                ..Default::default()
            },
        );

        // Should match: pipeline has egress in mesh AND destination matches ingress URL
        let result = PipelineExecutor::find_mesh_for_egress(
            "my-pipeline",
            "https://partner.example.com/api/users",
            &config,
        );
        assert!(result.is_some());
        let (mesh_name, _) = result.unwrap();
        assert_eq!(mesh_name, "partner-mesh");
    }

    #[test]
    fn test_find_mesh_for_egress_no_match_wrong_destination() {
        use crate::models::mesh::config::{Mesh, MeshEgress, MeshIngress, MeshProtocol, MeshProvider};

        let mut config = Config::default();

        config.ingress.insert(
            "partner-ingress".to_string(),
            MeshIngress {
                pipeline: "partner-pipeline".to_string(),
                ingress_type: MeshProtocol::Http,
                urls: vec!["https://partner.example.com/api".to_string()],
                enabled: true,
                ..Default::default()
            },
        );

        config.egress.insert(
            "my-egress".to_string(),
            MeshEgress {
                pipeline: "my-pipeline".to_string(),
                egress_type: MeshProtocol::Http,
                enabled: true,
                ..Default::default()
            },
        );

        config.mesh.insert(
            "partner-mesh".to_string(),
            Mesh {
                mesh_type: MeshProtocol::Http,
                provider: MeshProvider::Local,
                ingress: vec!["partner-ingress".to_string()],
                egress: vec!["my-egress".to_string()],
                enabled: true,
                ..Default::default()
            },
        );

        // Should NOT match: destination URL doesn't match ingress
        let result = PipelineExecutor::find_mesh_for_egress(
            "my-pipeline",
            "https://other.example.com/api/users",
            &config,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_find_mesh_for_egress_no_match_pipeline_not_in_mesh() {
        use crate::models::mesh::config::{Mesh, MeshEgress, MeshIngress, MeshProtocol, MeshProvider};

        let mut config = Config::default();

        config.ingress.insert(
            "partner-ingress".to_string(),
            MeshIngress {
                pipeline: "partner-pipeline".to_string(),
                ingress_type: MeshProtocol::Http,
                urls: vec!["https://partner.example.com/api".to_string()],
                enabled: true,
                ..Default::default()
            },
        );

        // Egress is for a DIFFERENT pipeline
        config.egress.insert(
            "other-egress".to_string(),
            MeshEgress {
                pipeline: "other-pipeline".to_string(),
                egress_type: MeshProtocol::Http,
                enabled: true,
                ..Default::default()
            },
        );

        config.mesh.insert(
            "partner-mesh".to_string(),
            Mesh {
                mesh_type: MeshProtocol::Http,
                provider: MeshProvider::Local,
                ingress: vec!["partner-ingress".to_string()],
                egress: vec!["other-egress".to_string()],
                enabled: true,
                ..Default::default()
            },
        );

        // Should NOT match: our pipeline doesn't have an egress in this mesh
        let result = PipelineExecutor::find_mesh_for_egress(
            "my-pipeline",
            "https://partner.example.com/api/users",
            &config,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_find_mesh_for_egress_disabled_mesh_ignored() {
        use crate::models::mesh::config::{Mesh, MeshEgress, MeshIngress, MeshProtocol, MeshProvider};

        let mut config = Config::default();

        config.ingress.insert(
            "partner-ingress".to_string(),
            MeshIngress {
                pipeline: "partner-pipeline".to_string(),
                ingress_type: MeshProtocol::Http,
                urls: vec!["https://partner.example.com/api".to_string()],
                enabled: true,
                ..Default::default()
            },
        );

        config.egress.insert(
            "my-egress".to_string(),
            MeshEgress {
                pipeline: "my-pipeline".to_string(),
                egress_type: MeshProtocol::Http,
                enabled: true,
                ..Default::default()
            },
        );

        config.mesh.insert(
            "partner-mesh".to_string(),
            Mesh {
                mesh_type: MeshProtocol::Http,
                provider: MeshProvider::Local,
                ingress: vec!["partner-ingress".to_string()],
                egress: vec!["my-egress".to_string()],
                enabled: false, // Disabled!
                ..Default::default()
            },
        );

        // Should NOT match: mesh is disabled
        let result = PipelineExecutor::find_mesh_for_egress(
            "my-pipeline",
            "https://partner.example.com/api/users",
            &config,
        );
        assert!(result.is_none());
    }
}
