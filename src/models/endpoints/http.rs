use std::collections::HashMap;
use serde::Deserialize;
use http::{Response};
use serde_json::Value;
use async_trait::async_trait;
use crate::config::config::ConfigError;
use crate::models::endpoints::endpoint_type::{EndpointHandler, EndpointType};
use crate::models::envelope::envelope::Envelope;
use crate::models::middleware::types::Error;
use crate::router::route_config::RouteConfig;

#[derive(Debug, Deserialize)]
pub struct HttpEndpoint {}

impl EndpointType for HttpEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is not empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .map_or(true, |s| s.trim().is_empty())
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
            RouteConfig {
                path: format!("{}/get-route", path_prefix), // Example route
                methods: vec![http::Method::GET],
                description: Some("Handles GET requests for HttpEndpoint".to_string()),
            },
            RouteConfig {
                path: format!("{}/post-route", path_prefix), // Example route
                methods: vec![http::Method::POST],
                description: Some("Handles POST requests for HttpEndpoint".to_string()),
            },
        ]
    }

}

#[async_trait]
impl EndpointHandler<Value> for HttpEndpoint {
    type ReqBody = Value;
    type ResBody = Value;

    async fn handle_request(
        &self,
        mut envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, crate::models::middleware::types::Error> {
        // Add or modify normalized data in the envelope
        envelope.normalized_data = Some(serde_json::json!({
            "message": "BasicEndpoint processed the request",
            "original_data": envelope.original_data
        }));

        Ok(envelope)
    }
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