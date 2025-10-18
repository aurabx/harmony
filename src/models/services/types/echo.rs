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
pub struct EchoEndpoint {}

#[async_trait]
impl ServiceType for EchoEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is non-empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(ConfigError::InvalidEndpoint {
                name: "echo".to_string(),
                reason: "Echo endpoint requires a non-empty 'path_prefix'".to_string(),
            });
        }
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Retrieve 'path_prefix' from the options or use a default value
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("/echo");

        vec![RouteConfig {
            path: format!("{}/{{*wildcard}}", path_prefix), // Use {*wildcard} syntax
            methods: vec![Method::POST],
            description: Some("Handles Echo POST requests".to_string()),
        }]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        // For HTTP protocol, delegate to HttpEndpoint for consistent HTTP parsing
        if ctx.protocol == crate::models::protocol::Protocol::Http {
            let http = crate::models::services::types::http::HttpEndpoint {};
            return http.build_protocol_envelope(ctx, options).await;
        }
        Err(crate::utils::Error::from(
            "JmixEndpoint only supports Protocol::Http envelope building",
        ))
    }
}

#[async_trait]
impl ServiceHandler<Value> for EchoEndpoint {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Capture subpath from request metadata inserted by dispatcher
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();

        // Add or modify the envelope's normalized data including subpath
        let full_path = envelope
            .request_details
            .metadata
            .get("full_path")
            .cloned()
            .unwrap_or_default();

        envelope.normalized_data = Some(serde_json::json!({
            "path": subpath,
            "full_path": full_path,
            "headers": envelope.request_details.headers,
            "original_data": envelope.original_data,
        }));

        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // Echo service generates response directly from request (no backend call)
        // Build echo data structure with path, headers, etc.
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();
        let full_path = envelope
            .request_details
            .metadata
            .get("full_path")
            .cloned()
            .unwrap_or_default();

        // Merge with existing normalized_data if present (to preserve middleware annotations)
        let mut echo_data = serde_json::json!({
            "path": subpath,
            "full_path": full_path,
            "headers": envelope.request_details.headers,
            "original_data": envelope.original_data,
        });

        if let Some(existing) = envelope.normalized_data {
            // Merge existing data into echo_data
            if let (Some(echo_map), Some(existing_map)) =
                (echo_data.as_object_mut(), existing.as_object())
            {
                for (k, v) in existing_map {
                    echo_map.insert(k.clone(), v.clone());
                }
            }
        }

        envelope.normalized_data = Some(echo_data);

        // Build ResponseEnvelope with normalized_data from the request
        let status = 200;
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let body_json =
            serde_json::to_vec(&envelope.normalized_data).unwrap_or_else(|_| Vec::new());

        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            headers,
            body_json,
            None,
        );

        // Set normalized_data to the echoed content
        response_envelope.normalized_data = envelope.normalized_data;

        Ok(response_envelope)
    }

    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Add protocol metadata for echo service
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));
        envelope
            .response_details
            .metadata
            .insert("service".to_string(), "echo".to_string());
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
            let body_str = serde_json::to_string(&normalized)
                .map_err(|_| Error::from("Failed to serialize Echo response JSON"))?;
            Body::from(body_str)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct Echo HTTP response"))
    }
}
