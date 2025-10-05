use std::collections::HashMap;
use async_trait::async_trait;
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::Envelope;
use crate::models::services::services::{ServiceHandler, ServiceType};
use serde_json::Value;
use serde::Deserialize;
use crate::router::route_config::RouteConfig;
use http::{Method, Response};
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
    type ResBody = Value;

    // Process the incoming request and transform it into an Envelope
    async fn transform_request(
        &self,
        mut envelope: Envelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, Error> {
        // Capture subpath from request metadata inserted by dispatcher
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();

        // Add or modify the envelope's normalized data
        envelope.normalized_data = Some(serde_json::json!({
            "message": "FHIR endpoint received the request",
            "path": subpath,
            "original_data": envelope.original_data,
        }));

        Ok(envelope)
    }

    // Convert the processed Envelope into an HTTP Response
    async fn transform_response(
        &self,
        envelope: Envelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response<Self::ResBody>, Error> {
        // Return the envelope's normalized data directly as a JSON Value
        let body: serde_json::Value = envelope.normalized_data.unwrap_or(serde_json::Value::Null);
        Response::builder()
            .status(200)
            .body(body)
            .map_err(|_| Error::from("Failed to construct FHIR HTTP response"))
    }
}
