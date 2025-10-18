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
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // FHIR passthrough - convert request to response with 200 OK
        // @todo In a real implementation, this would make a FHIR API call to a backend
        let status = 200;
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            "application/fhir+json".to_string(),
        );

        let body = if let Some(ref normalized) = envelope.normalized_data {
            serde_json::to_vec(normalized).unwrap_or_default()
        } else {
            envelope.original_data.clone()
        };

        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            headers,
            body,
            None,
        );

        response_envelope.normalized_data = envelope.normalized_data;

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
