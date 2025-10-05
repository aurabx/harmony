use std::collections::HashMap;
use async_trait::async_trait;
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::services::services::{ServiceHandler, ServiceType};
use serde_json::Value;
use serde::Deserialize;
use crate::router::route_config::RouteConfig;
use http::Method;
use axum::{response::Response, body::Body};
use crate::utils::Error;

#[derive(Debug, Deserialize)]
pub struct FhirEndpoint {}

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
        vec![
            RouteConfig {
                path: format!("{}/{{*wildcard}}", path_prefix), // Use {*wildcard} syntax
                methods: vec![Method::GET, Method::POST, Method::PUT, Method::DELETE],
                description: Some("Handles FHIR GET/POST/PUT/DELETE requests".to_string()),
            },
        ]
    }
}

#[async_trait]
impl ServiceHandler<Value> for FhirEndpoint {
    type ReqBody = Value;

    // Process the incoming request and transform it into an Envelope
    async fn transform_request(
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

        // Add or modify the envelope's normalized data
        let full_path = envelope
            .request_details
            .metadata
            .get("full_path")
            .cloned()
            .unwrap_or_default();

        envelope.normalized_data = Some(serde_json::json!({
            "message": "FHIR endpoint received the request",
            "path": subpath,
            "full_path": full_path,
            "headers": envelope.request_details.headers,
            "original_data": envelope.original_data,
        }));

        Ok(envelope)
    }

    // Convert the processed Envelope into an HTTP Response
    async fn transform_response(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        let nd = envelope.normalized_data.unwrap_or(serde_json::Value::Null);
        let response_meta = nd.get("response");

        let status = response_meta
            .and_then(|m| m.get("status"))
            .and_then(|s| s.as_u64())
            .and_then(|code| http::StatusCode::from_u16(code as u16).ok())
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);
        let mut has_content_type = false;
        if let Some(hdrs) = response_meta.and_then(|m| m.get("headers")).and_then(|h| h.as_object()) {
            for (k, v) in hdrs.iter() {
                if let Some(val_str) = v.as_str() {
                    if k.eq_ignore_ascii_case("content-type") { has_content_type = true; }
                    builder = builder.header(k.as_str(), val_str);
                }
            }
        }

        if let Some(body_str) = response_meta.and_then(|m| m.get("body")).and_then(|b| b.as_str()) {
            return builder
                .body(Body::from(body_str.to_string()))
                .map_err(|_| Error::from("Failed to construct FHIR HTTP response"));
        }

        let body_str = serde_json::to_string(&nd).map_err(|_| Error::from("Failed to serialize FHIR response JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct FHIR HTTP response"))
    }
}
