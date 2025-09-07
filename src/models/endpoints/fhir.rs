use std::collections::HashMap;
use async_trait::async_trait;
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::Envelope;
use crate::models::middleware::types::Error;
use crate::models::endpoints::endpoint_type::{EndpointHandler, EndpointType};
use serde_json::Value;
use serde::Deserialize;
use crate::router::route_config::RouteConfig;
use http::{Method, Response};

#[derive(Debug, Deserialize)]
pub struct FhirEndpoint {}

impl EndpointType for FhirEndpoint {
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

        // Return route configurations for GET and POST methods
        vec![
            RouteConfig {
                path: format!("{}/:path", path_prefix),
                methods: vec![Method::GET, Method::POST],
                description: Some("Handles FHIR GET and POST requests".to_string()),
            },
        ]
    }
}

#[async_trait]
impl EndpointHandler<Value> for FhirEndpoint {
    type ReqBody = Value;
    type ResBody = Value;

    // Process the incoming request and transform it into an Envelope
    async fn handle_request(
        &self,
        mut envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, crate::models::middleware::types::Error> {
        // Add or modify the envelope's normalized data
        envelope.normalized_data = Some(serde_json::json!({
            "message": "FHIR endpoint received the request",
            "original_data": envelope.original_data,
        }));

        Ok(envelope)
    }

    // Convert the processed Envelope into an HTTP Response
    async fn handle_response(
        &self,
        envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Response<Self::ResBody>, crate::models::middleware::types::Error> {
        // Serialize the envelope's normalized data into an HTTP Response
        let body = serde_json::to_string(&envelope.normalized_data).map_err(|_| {
            Error::from("Failed to serialize FHIR response payload into JSON")
        })?;
        Response::builder()
            .status(200)
            .body(body.into())
            .map_err(|_| Error::from("Failed to construct FHIR HTTP response"))
    }
}

// impl FhirEndpoint {
//     /// Example handler for FHIR GET or POST requests
//     async fn fhir_handler(JsonExtract(payload): JsonExtract<Value>) -> Json<Value> {
//         Json(serde_json::json!({
//             "status": "success",
//             "message": "FHIR endpoint accepted the request",
//             "payload": payload,
//         }))
//     }
// }