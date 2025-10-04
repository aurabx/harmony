use std::collections::HashMap;
use async_trait::async_trait;
use axum::{http::{Response}};
use serde_json::Value;
use serde::Deserialize;
use crate::models::envelope::envelope::Envelope;
use crate::utils::Error;
use crate::config::config::ConfigError;
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use http::Method;

#[derive(Debug, Deserialize)]
pub struct JmixEndpoint {}

impl ServiceType for JmixEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is non-empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .map_or(true, |s| s.trim().is_empty())
        {
            return Err(ConfigError::InvalidEndpoint {
                name: "jmix".to_string(),
                reason: "Jmix endpoint requires a non-empty 'path_prefix'".to_string(),
            });
        }
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Retrieve 'path_prefix' from the options or use a default value
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("/jmix");

        vec![
            RouteConfig {
                path: format!("{}/{{*wildcard}}", path_prefix), // Use {*wildcard} syntax
                methods: vec![Method::POST],
                description: Some("Handles JMIX POST requests".to_string()),
            },
            RouteConfig {
                path: format!("{}/{{*wildcard}}", path_prefix), // Use {*wildcard} syntax
                methods: vec![Method::GET],
                description: Some("Handles JMIX GET requests".to_string()),
            }
        ]
    }
}

#[async_trait]
impl ServiceHandler<Value> for JmixEndpoint {
    type ReqBody = Value;
    type ResBody = Value;

    async fn transform_request(
        &self,
        mut envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, Error> {
        let path = options.get("path").cloned().unwrap_or_default();

        // Add or modify normalized data in the envelope
        envelope.normalized_data = Some(serde_json::json!({
            "message": "Jmix endpoint processed the request",
            "path": path,
            "original_data": envelope.original_data,
        }));

        Ok(envelope)
    }

    async fn transform_response(
        &self,
        envelope: Envelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response<Self::ResBody>, Error> {
        // Serialize the normalized data into a JSON HTTP response
        let body = serde_json::to_string(&envelope.normalized_data).map_err(|_| {
            Error::from("Failed to serialize Jmix response payload into JSON")
        })?;
        Response::builder()
            .status(200)
            .body(body.into())
            .map_err(|_| Error::from("Failed to construct JMIX HTTP response"))
    }
}
