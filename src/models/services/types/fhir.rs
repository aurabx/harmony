use crate::config::config::ConfigError;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct FhirEndpoint {}

#[async_trait]
impl ServiceType for FhirEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is valid
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if path_prefix.trim().is_empty() {
            return Err(ConfigError::InvalidEndpoint {
                name: "fhir".to_string(),
                reason: "FHIR endpoint requires a non-empty 'path_prefix'".to_string(),
            });
        }

        // Optionally validate other fields from `options` as needed
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Get the 'path_prefix' from options or default to "/fhir"
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("/fhir");

        // Return route configurations for GET/POST/PUT/DELETE methods
        vec![RouteConfig {
            path: format!("{}/{{*wildcard}}", path_prefix), // Use {*wildcard} syntax
            methods: vec![Method::GET, Method::POST, Method::PUT, Method::DELETE],
            description: Some("Handles FHIR GET/POST/PUT/DELETE requests".to_string()),
        }]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        // Delegate to HttpEndpoint for HTTP variant
        let http = crate::models::services::types::http::HttpEndpoint {};
        http.build_protocol_envelope(ctx, options).await
    }
}

#[async_trait]
impl ServiceHandler<Value> for FhirEndpoint {
    type ReqBody = Value;

    // Process the incoming request and transform it into an Envelope
    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
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
            .ok_or_else(|| Error::from("FHIR backend requires 'base_url' in options"))?;
        
        // Check if middleware has already set target_details
        let target_details = if let Some(existing_target) = envelope.target_details.take() {
            // Middleware has set target_details - use it
            // But merge base_url from backend options if not set by middleware
            let mut target = existing_target;
            if target.base_url.is_empty() {
                target.base_url = base_url.to_string();
            }
            tracing::debug!("FHIR backend using middleware-provided target_details");
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
            
            // Ensure FHIR-specific content type is set if not present
            target.headers
                .entry("accept".to_string())
                .or_insert_with(|| "application/fhir+json".to_string());
            
            target
        };
        
        tracing::debug!(
            "FHIR backend targeting: {} {}", 
            target_details.method, 
            target_details.full_url().unwrap_or_else(|_| "<invalid-url>".to_string())
        );
        
        // Store target_details in envelope for future use (Targets model, etc.)
        envelope.target_details = Some(target_details.clone());
        
        // Make the actual HTTP request to the FHIR server
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
        
        tracing::debug!("Sending FHIR request to: {}", full_url);
        
        // Execute the request
        let response = request_builder
            .send()
            .await
            .map_err(|e| Error::from(format!("FHIR request failed: {}", e)))?;
        
        let status = response.status().as_u16();
        tracing::debug!("FHIR backend response status: {}", status);
        
        // Extract response headers
        let mut response_headers = HashMap::new();
        for (key, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                response_headers.insert(key.to_string(), value_str.to_string());
            }
        }
        
        // Ensure FHIR content type is set in response
        response_headers
            .entry("content-type".to_string())
            .or_insert_with(|| "application/fhir+json".to_string());
        
        // Get response body
        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| Error::from(format!("Failed to read FHIR response body: {}", e)))?
            .to_vec();
        
        tracing::debug!("FHIR backend response body size: {} bytes", body_bytes.len());
        
        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            response_headers,
            body_bytes,
            None,
        );
        
        // Try to parse response as JSON if content-type indicates FHIR or JSON
        if let Some(content_type) = response_envelope.response_details.headers.get("content-type") {
            if content_type.contains("application/fhir+json") || content_type.contains("application/json") {
                if let Ok(json_value) = serde_json::from_slice::<serde_json::Value>(&response_envelope.original_data) {
                    response_envelope.normalized_data = Some(json_value);
                }
            }
        }
        
        Ok(response_envelope)
    }

    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Add protocol metadata and ensure FHIR content-type is set
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));
        envelope
            .response_details
            .metadata
            .insert("service".to_string(), "fhir".to_string());
        
        // Ensure FHIR content-type is present for HTTP
        if ctx.protocol == crate::models::protocol::Protocol::Http {
            envelope
                .response_details
                .headers
                .entry("content-type".to_string())
                .or_insert_with(|| "application/fhir+json".to_string());
        }
        Ok(())
    }

    // Convert the processed ResponseEnvelope into an HTTP Response
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
                .map_err(|_| Error::from("Failed to serialize FHIR response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct FHIR HTTP response"))
    }
}
