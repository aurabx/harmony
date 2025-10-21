use crate::config::config::ConfigError;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct HttpEndpoint {}

#[async_trait]
impl ServiceType for HttpEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is not empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(ConfigError::InvalidEndpoint {
                name: "basic".to_string(),
                reason: "Basic endpoint requires a non-empty 'path_prefix'".to_string(),
            });
        }
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("/");

        vec![
            // Handle exact path match
            RouteConfig {
                path: path_prefix.to_string(),
                methods: vec![
                    http::Method::GET,
                    http::Method::POST,
                    http::Method::PUT,
                    http::Method::DELETE,
                ],
                description: Some("Handles HTTP requests at exact path".to_string()),
            },
            // Handle subpaths (e.g., /dicom/echo, /api/v1/users)
            RouteConfig {
                path: format!("{}/{{*wildcard}}", path_prefix),
                methods: vec![
                    http::Method::GET,
                    http::Method::POST,
                    http::Method::PUT,
                    http::Method::DELETE,
                ],
                description: Some("Handles HTTP requests with subpaths".to_string()),
            },
        ]
    }

    // noinspection DuplicatedCode
    // Protocol-agnostic builder (HTTP variant)
    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        use crate::models::envelope::envelope::RequestEnvelope;
        use crate::utils::Error;
        use std::collections::HashMap as Map;

        if ctx.protocol != crate::models::protocol::Protocol::Http {
            return Err(Error::from(
                "HttpEndpoint only supports Protocol::Http in build_protocol_envelope",
            ));
        }

        let attrs = ctx
            .attrs
            .as_object()
            .ok_or_else(|| Error::from("invalid attrs for HTTP"))?;
        let headers_map: Map<String, String> = attrs
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let cookies_map: Map<String, String> = attrs
            .get("cookies")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let query_params: Map<String, Vec<String>> = attrs
            .get("query_params")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let vec = v
                            .as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>();
                        (k.clone(), vec)
                    })
                    .collect::<Map<String, Vec<String>>>()
            })
            .unwrap_or_default();
        let cache_status = attrs
            .get("cache_status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut metadata: Map<String, String> = Map::new();
        // pass through HTTP-derived meta (path, full_path) from ctx.meta
        if let Some(path) = ctx.meta.get("path") {
            metadata.insert("path".into(), path.clone());
        }
        if let Some(full) = ctx.meta.get("full_path") {
            metadata.insert("full_path".into(), full.clone());
        }
        if let Some(proto) = ctx.meta.get("protocol") {
            metadata.insert("protocol".into(), proto.clone());
        }

        let method = attrs
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let uri = attrs
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Try to parse payload as JSON for normalized_data
        let normalized_data = serde_json::from_slice(&ctx.payload).ok();

        RequestEnvelope::builder()
            .method(method)
            .uri(uri)
            .headers(headers_map)
            .cookies(cookies_map)
            .query_params(query_params)
            .cache_status(cache_status)
            .metadata(metadata)
            .target_details(None)
            .original_data(ctx.payload)
            .normalized_data(normalized_data)
            .normalized_snapshot(None)
            .build()
    }
}

#[async_trait]
impl ServiceHandler<Value> for HttpEndpoint {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Populate normalized data with real request context

        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        use crate::models::envelope::envelope::TargetDetails;
        
        // Extract base_url from backend options
        let base_url = options
            .get("base_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::from("HTTP backend requires 'base_url' in options"))?;
        
        // Check if middleware has already set target_details
        let target_details = if let Some(existing_target) = envelope.target_details.take() {
            // Middleware has set target_details - use it
            // But merge base_url from backend options if not set by middleware
            let mut target = existing_target;
            if target.base_url.is_empty() {
                target.base_url = base_url.to_string();
            }
            tracing::debug!("HTTP backend using middleware-provided target_details");
            target
        } else {
            // No target_details set by middleware - create from request_details
            // Use the path from metadata (without endpoint prefix) if available,
            // otherwise fall back to the full URI
            let path = envelope
                .request_details
                .metadata
                .get("path")
                .map(|p| format!("/{}", p))
                .unwrap_or_else(|| envelope.request_details.uri.clone());
            
            // Create TargetDetails from request_details with base_url
            let mut target = TargetDetails::from_request_details(
                base_url.to_string(),
                &envelope.request_details
            );
            
            // Override URI with the stripped path
            target.uri = path;
            target
        };
        
        tracing::debug!(
            "HTTP backend targeting: {} {}", 
            target_details.method, 
            target_details.full_url().unwrap_or_else(|_| "<invalid-url>".to_string())
        );
        
        // Store target_details in envelope for future use (Targets model, etc.)
        envelope.target_details = Some(target_details.clone());
        
        // Make the actual HTTP request
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
        
        // Add headers from target_details
        for (key, value) in &target_details.headers {
            request_builder = request_builder.header(key, value);
        }
        
        // Add request body if present
        if !envelope.original_data.is_empty() {
            request_builder = request_builder.body(envelope.original_data.clone());
        }
        
        tracing::debug!("Sending HTTP request to: {}", full_url);
        
        // Execute the request
        let response = request_builder
            .send()
            .await
            .map_err(|e| Error::from(format!("HTTP request failed: {}", e)))?;
        
        let status = response.status().as_u16();
        tracing::debug!("HTTP backend response status: {}", status);
        
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
            .map_err(|e| Error::from(format!("Failed to read response body: {}", e)))?
            .to_vec();
        
        tracing::debug!("HTTP backend response body size: {} bytes", body_bytes.len());
        
        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            response_headers,
            body_bytes,
            None,
        );
        
        // Try to parse response as JSON if content-type indicates JSON
        if let Some(content_type) = response_envelope.response_details.headers.get("content-type") {
            if content_type.contains("application/json") {
                if let Ok(json_value) = serde_json::from_slice::<serde_json::Value>(&response_envelope.original_data) {
                    response_envelope.normalized_data = Some(json_value);
                }
            }
        }
        
        Ok(response_envelope)
    }

    /// Protocol-aware response post-processing
    /// 
    /// For HTTP service, this adds protocol metadata to response headers
    /// to help with debugging and observability.
    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Add protocol information to response metadata for observability
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));
        
        // For HTTP protocol, optionally add X-Protocol header for debugging
        if ctx.protocol == crate::models::protocol::Protocol::Http {
            envelope
                .response_details
                .headers
                .entry("x-harmony-protocol".to_string())
                .or_insert_with(|| "http".to_string());
        }
        
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Build response from ResponseEnvelope
        let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        // Add headers from response_details
        for (k, v) in &envelope.response_details.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        // Use original_data if available, otherwise serialize normalized_data
        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else if let Some(normalized) = envelope.normalized_data {
            let body_bytes = serde_json::to_vec(&normalized)
                .map_err(|_| Error::from("Failed to serialize HTTP response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct HTTP response"))
    }
}
