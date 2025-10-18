use crate::config::config::Config;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
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
    /// 2. Incoming middleware chain (left)
    /// 3. Backend invocation
    /// 4. Outgoing middleware chain (right)
    /// 5. Endpoint service post-processing (protocol-aware)
    /// 6. Return ResponseEnvelope
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

        // 2. Incoming middleware chain (left)
        let envelope = Self::process_incoming_middleware(envelope, pipeline, config).await?;

        // 3. Backend invocation
        let response = Self::process_backends(envelope, pipeline, config).await?;

        // 4. Outgoing middleware chain (right)
        let mut response = Self::process_outgoing_middleware(response, pipeline, config).await?;

        // 5. Endpoint service post-processing (protocol-aware)
        Self::process_endpoint_outgoing(&mut response, pipeline, config, ctx).await?;

        tracing::info!("Pipeline execution completed successfully");
        Ok(response)
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

        let endpoint = config
            .endpoints
            .get(endpoint_name)
            .ok_or_else(|| {
                PipelineError::ConfigError(format!("Endpoint '{}' not found", endpoint_name))
            })?;

        let service = endpoint
            .resolve_service()
            .map_err(|e| PipelineError::ServiceError(format!("Failed to resolve service: {}", e)))?;

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
        tracing::debug!(
            "Processing incoming middleware for {} middlewares",
            pipeline.middleware.len()
        );

        // Convert to JSON envelope for middleware processing
        let normalized_data = envelope.normalized_data.clone();
        let json_envelope = RequestEnvelope {
            request_details: envelope.request_details.clone(),
            backend_request_details: envelope.backend_request_details.clone(),
            original_data: normalized_data.unwrap_or_else(|| {
                serde_json::from_slice(&envelope.original_data).unwrap_or(serde_json::Value::Null)
            }),
            normalized_data: envelope.normalized_data.clone(),
            normalized_snapshot: envelope.normalized_snapshot.clone(),
        };

        // Build middleware instances
        let middleware_instances =
            build_middleware_instances_for_pipeline(&pipeline.middleware, config).map_err(
                |err| PipelineError::MiddlewareError(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    err,
                ))),
            )?;

        let middleware_chain = MiddlewareChain::new(middleware_instances);

        // Process through middleware chain
        let processed_json_envelope = middleware_chain
            .left(json_envelope)
            .await
            .map_err(PipelineError::MiddlewareError)?;

        // Convert back to Vec<u8> envelope
        let processed_envelope = RequestEnvelope {
            request_details: processed_json_envelope.request_details,
            backend_request_details: processed_json_envelope.backend_request_details,
            original_data: envelope.original_data,
            normalized_data: processed_json_envelope.normalized_data,
            normalized_snapshot: processed_json_envelope.normalized_snapshot,
        };

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
            tracing::info!("Skipping backends due to endpoint 'skip_backends' flag");
            // Return empty response
            return Ok(ResponseEnvelope::from_backend(
                envelope.request_details.clone(),
                200,
                HashMap::new(),
                Vec::new(),
                None,
            ));
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
        tracing::debug!(
            "Processing outgoing middleware for {} middlewares",
            pipeline.middleware.len()
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
            build_middleware_instances_for_pipeline(&pipeline.middleware, config).map_err(
                |err| PipelineError::MiddlewareError(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    err,
                ))),
            )?;

        let middleware_chain = MiddlewareChain::new(middleware_instances);

        // Process through middleware chain (right side)
        let processed_json_envelope = middleware_chain
            .right(json_envelope)
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

        let endpoint = config
            .endpoints
            .get(endpoint_name)
            .ok_or_else(|| {
                PipelineError::ConfigError(format!("Endpoint '{}' not found", endpoint_name))
            })?;

        let service = endpoint
            .resolve_service()
            .map_err(|e| PipelineError::ServiceError(format!("Failed to resolve service: {}", e)))?;

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
}
