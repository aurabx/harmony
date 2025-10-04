use std::collections::HashMap;
use async_trait::async_trait;
use serde_json::Value;
use serde::Deserialize;
use http::{Response};
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::Envelope;
use crate::models::services::services::{ServiceType, ServiceHandler};
use crate::router::route_config::RouteConfig;
use http::Method;
use crate::utils::Error;

#[derive(Debug, Deserialize)]
pub struct EchoEndpoint {}

impl ServiceType for EchoEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is non-empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .map_or(true, |s| s.trim().is_empty())
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

        vec![
            RouteConfig {
                path: format!("{}/{{*wildcard}}", path_prefix), // Use {*wildcard} syntax
                methods: vec![Method::POST],
                description: Some("Handles Echo POST requests".to_string()),
            },
        ]
    }
}

#[async_trait]
impl ServiceHandler<Value> for EchoEndpoint {
    type ReqBody = Value;
    type ResBody = Value;

    async fn transform_request(
        &self,
        mut envelope: Envelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, Error> {
        // Add or modify the envelope's normalized data
        envelope.normalized_data = Some(serde_json::json!({
            "message": "Echo endpoint received the request",
            "original_data": envelope.original_data,
        }));

        Ok(envelope)
    }

    async fn transform_response(
        &self,
        envelope: Envelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response<Self::ResBody>, Error> {
        // Serialize the envelope's normalized data into an HTTP Response
        let body = serde_json::to_string(&envelope.normalized_data).map_err(|_| {
            Error::from("Failed to serialize Echo response payload into JSON")
        })?;
        Response::builder()
            .status(200)
            .body(body.into())
            .map_err(|_| Error::from("Failed to construct Echo HTTP response"))
    }
}
