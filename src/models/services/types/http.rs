use std::collections::HashMap;
use serde::Deserialize;
use http::{Response};
use serde_json::Value;
use async_trait::async_trait;
use crate::config::config::ConfigError;
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::models::envelope::envelope::Envelope;
use crate::utils::Error;
use crate::router::route_config::RouteConfig;

#[derive(Debug, Deserialize)]
pub struct HttpEndpoint {}

impl ServiceType for HttpEndpoint {
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
                path: format!("{}/{{*wildcard}}", path_prefix),
                methods: vec![
                    http::Method::GET,
                    http::Method::POST,
                    http::Method::PUT,
                    http::Method::DELETE,
                ],
                description: Some("Handles GET/POST/PUT/DELETE for HttpEndpoint".to_string()),
            },
        ]
    }

}

#[async_trait]
impl ServiceHandler<Value> for HttpEndpoint {
    type ReqBody = Value;
    type ResBody = Value;

    async fn transform_request(
        &self,
        mut envelope: Envelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, Error> {
        // Add or modify normalized data in the envelope
        envelope.normalized_data = Some(serde_json::json!({
            "message": "BasicEndpoint processed the request",
            "original_data": envelope.original_data
        }));

        Ok(envelope)
    }
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
            .map_err(|_| Error::from("Failed to construct HTTP response"))
    }
}