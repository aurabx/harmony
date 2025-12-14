use crate::config::config::Config;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope, TargetDetails};
use crate::models::middleware::chain::MiddlewareChain;
use crate::models::middleware::middleware::build_middleware_instances_for_pipeline;
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
    /// * `pipeline` - Pipeline configuration (endpoints, backends, middleware)
    /// * `config` - Full application configuration
    /// * `ctx` - Protocol context for protocol-specific metadata
    ///
    /// # Returns
    /// ResponseEnvelope on success, PipelineError on failure
    #[tracing::instrument(skip(envelope, pipeline, config, ctx), fields(
        protocol = ?ctx.protocol,
        pipeline = pipeline.description.as_str()
    ))]
    pub async fn execute(
        envelope: RequestEnvelope<Vec<u8>>,
        pipeline: &Pipeline,
        config: &Config,
        ctx: &ProtocolCtx,
    ) -> Result<ResponseEnvelope<Vec<u8>>, PipelineError> {
        tracing::info!("Executing pipeline for protocol: {:?}", ctx.protocol);

        // 1. Endpoint service preprocessing
        let envelope = Self::process_endpoint_incoming(envelope, pipeline, config).await?;

        // 2. Resolve target_details from backend config (so middleware has access)
        let envelope = Self::resolve_target_details(envelope, pipeline, config).await?;

        // 3. Incoming middleware chain (left)
        let envelope = Self::process_incoming_middleware(envelope, pipeline, config).await?;

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
    async fn process_incoming_middleware(
        envelope: RequestEnvelope<Vec<u8>>,
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

        // Build middleware instances
        let middleware_instances =
            build_middleware_instances_for_pipeline(&left_chain, config).map_err(
                |err| {
                    PipelineError::MiddlewareError(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        err,
                    )))
                },
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
}
