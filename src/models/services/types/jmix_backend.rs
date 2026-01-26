use crate::config::config::ConfigError;
use crate::models::connection::ConnectionConfig;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope, TargetDetails};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::response::Response;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// JMIX Backend Service
///
/// Forwards JMIX requests (GET/POST) to upstream JMIX servers.
/// Supports:
/// - POST /api/jmix (upload envelope)
/// - GET /api/jmix/{id} (retrieve envelope by ID)
/// - GET /api/jmix/{id}/manifest (retrieve manifest)
/// - GET /api/jmix?studyInstanceUid=... (query by study UID)
#[derive(Debug, Deserialize)]
pub struct JmixBackend {}

#[async_trait]
impl ServiceType for JmixBackend {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate that connection config is present
        let has_connection = options.get("connection").is_some();
        let has_base_url = options
            .get("base_url")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);

        if !has_connection && !has_base_url {
            return Err(ConfigError::InvalidBackend {
                name: "jmix_backend".to_string(),
                reason: "JMIX backend requires 'connection' config or 'base_url' option"
                    .to_string(),
            });
        }

        Ok(())
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Backends don't define routes - they only handle outbound requests
        vec![]
    }

    async fn build_protocol_envelope(
        &self,
        _ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "JmixBackend does not support build_protocol_envelope (backend-only service)",
        ))
    }
}

#[async_trait]
impl ServiceHandler<Value> for JmixBackend {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Backends don't process incoming requests from clients
        // This path is only used when backend is mistakenly configured as an endpoint
        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // Parse connection config if available
        let connection_config = options
            .get("connection")
            .and_then(|v| serde_json::from_value::<ConnectionConfig>(v.clone()).ok());

        // Extract base_url from backend options or connection config
        let base_url = if let Some(ref conn) = connection_config {
            conn.to_base_url()
        } else {
            options
                .get("base_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };

        if base_url.is_empty() {
            return Err(Error::from(
                "JMIX backend requires 'base_url' in options or valid 'connection' config",
            ));
        }

        // Use target_details from executor (already populated from request_details + backend config)
        // Fill in base_url if not already set by executor
        let mut target_details = if let Some(mut target) = envelope.target_details.take() {
            if target.base_url.is_empty() {
                target.base_url = base_url.to_string();
            }
            target
        } else {
            // Fallback: create from request_details (shouldn't normally happen)
            let path = crate::models::services::path_utils::extract_path(&envelope);
            let mut target = TargetDetails::from_request_details(
                base_url.to_string(),
                &envelope.request_details,
            );
            target.uri = path;
            target
        };

        // Apply path_prefix from backend options if present
        // This prepends a path to the request URI (e.g., "/api/jmix" + "/123" = "/api/jmix/123")
        if let Some(path_prefix) = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            let prefix = path_prefix.trim_end_matches('/');
            let uri = if target_details.uri.starts_with('/') {
                &target_details.uri
            } else {
                // Ensure uri has leading slash
                target_details.uri = format!("/{}", target_details.uri);
                &target_details.uri
            };
            target_details.uri = format!("{}{}", prefix, uri);
            tracing::debug!(
                "JMIX backend applied path_prefix '{}' to URI: {}",
                prefix,
                target_details.uri
            );
        }

        tracing::debug!(
            "JMIX backend targeting: {} {}",
            target_details.method,
            target_details
                .full_url()
                .unwrap_or_else(|_| "<invalid-url>".to_string())
        );

        // Store target_details in envelope for future use
        envelope.target_details = Some(target_details.clone());

        // Make the HTTP request to upstream JMIX server
        let (status, response_headers, body_bytes) =
            self.make_jmix_request(&target_details, &envelope, options)
                .await?;

        tracing::debug!(
            "JMIX backend response status: {}, body size: {} bytes",
            status,
            body_bytes.len()
        );

        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            response_headers,
            body_bytes,
            None,
        );

        // Try to parse response as JSON if content-type indicates JSON
        if let Some(content_type) = response_envelope
            .response_details
            .headers
            .get("content-type")
        {
            if content_type.contains("application/json") {
                if let Ok(json_value) =
                    serde_json::from_slice::<serde_json::Value>(&response_envelope.original_data)
                {
                    response_envelope.normalized_data = Some(json_value);
                }
            }
        }

        Ok(response_envelope)
    }

    async fn endpoint_outgoing_protocol(
        &self,
        _envelope: &mut ResponseEnvelope<Vec<u8>>,
        _ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Backends don't process outgoing protocol responses
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Backends don't construct final HTTP responses
        // This is handled by the endpoint service
        // Return a basic response as fallback
        use axum::body::Body;

        let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        for (k, v) in &envelope.response_details.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct JMIX backend response"))
    }
}

/// HTTP request helper for JmixBackend
impl JmixBackend {
    /// Make an HTTP request to upstream JMIX server
    async fn make_jmix_request(
        &self,
        target_details: &TargetDetails,
        envelope: &RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<(u16, HashMap<String, String>, Vec<u8>), Error> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::from(format!("Failed to create HTTP client: {}", e)))?;

        let full_url = target_details.full_url()?;

        // Build the request
        let mut request_builder = match target_details.method.as_str() {
            "GET" => client.get(&full_url),
            "POST" => client.post(&full_url),
            "PUT" => client.put(&full_url),
            "DELETE" => client.delete(&full_url),
            "PATCH" => client.patch(&full_url),
            "HEAD" => client.head(&full_url),
            method => {
                return Err(Error::from(format!("Unsupported HTTP method: {}", method)));
            }
        };

        // Apply authentication if present in backend options
        request_builder = crate::models::services::backend_auth::apply_backend_authentication(
            request_builder,
            options,
            "JMIX",
        );

        // Add headers from target_details, but drop hop-by-hop and Host headers
        for (key, value) in &target_details.headers {
            let k = key.to_ascii_lowercase();
            if matches!(
                k.as_str(),
                "host"
                    | "connection"
                    | "keep-alive"
                    | "proxy-connection"
                    | "transfer-encoding"
                    | "upgrade"
                    | "content-length"
            ) {
                continue; // let reqwest set correct values from URL/body
            }
            request_builder = request_builder.header(key, value);
        }

        // Add request body if present (for POST with ZIP uploads)
        if !envelope.original_data.is_empty() {
            request_builder = request_builder.body(envelope.original_data.clone());
        }

        tracing::debug!("Sending JMIX request to upstream: {}", full_url);

        // Execute the request
        let response = request_builder
            .send()
            .await
            .map_err(|e| Error::from(format!("JMIX request to upstream failed: {}", e)))?;

        let status = response.status().as_u16();
        tracing::debug!("JMIX upstream response status: {}", status);

        // Extract response headers
        let mut response_headers = HashMap::new();
        for (key, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                response_headers.insert(key.to_string(), value_str.to_string());
            }
        }

        // Get response body
        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| Error::from(format!("Failed to read JMIX response body: {}", e)))?
            .to_vec();

        Ok((status, response_headers, body_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_with_connection() {
        let backend = JmixBackend {};
        let mut options = HashMap::new();
        let connection = ConnectionConfig {
            host: "upstream.example.com".to_string(),
            port: Some(443),
            protocol: Some("https".to_string()),
            ..Default::default()
        };
        options.insert(
            "connection".to_string(),
            serde_json::to_value(connection).unwrap(),
        );
        assert!(backend.validate(&options).is_ok());
    }

    #[test]
    fn test_validate_with_base_url() {
        let backend = JmixBackend {};
        let mut options = HashMap::new();
        options.insert(
            "base_url".to_string(),
            serde_json::json!("https://upstream.example.com"),
        );
        assert!(backend.validate(&options).is_ok());
    }

    #[test]
    fn test_validate_missing_both() {
        let backend = JmixBackend {};
        let options = HashMap::new();
        assert!(backend.validate(&options).is_err());
    }

    #[test]
    fn test_build_router_empty() {
        let backend = JmixBackend {};
        let options = HashMap::new();
        let routes = backend.build_router(&options);
        assert!(routes.is_empty(), "Backends should not define routes");
    }
}
