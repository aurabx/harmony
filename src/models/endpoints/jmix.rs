use std::collections::HashMap;
use async_trait::async_trait;
use axum::{http::{Response}};
use serde_json::Value;
use serde::Deserialize;
use crate::models::envelope::envelope::Envelope;
use crate::models::middleware::types::Error;
use crate::config::config::ConfigError;
use crate::models::endpoints::endpoint_type::{EndpointHandler, EndpointType};
use crate::router::route_config::RouteConfig;
use http::Method;

#[derive(Debug, Deserialize)]
pub struct JmixEndpoint {}

impl EndpointType for JmixEndpoint {
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
                path: format!("{}/:path", path_prefix),
                methods: vec![Method::POST],
                description: Some("Handles JMIX POST requests".to_string()),
            },
            RouteConfig {
                path: format!("{}/:path", path_prefix),
                methods: vec![Method::GET],
                description: Some("Handles JMIX GET requests".to_string()),
            }
        ]
    }
}

#[async_trait]
impl EndpointHandler<Value> for JmixEndpoint {
    type ReqBody = Value;
    type ResBody = Value;

    async fn handle_request(
        &self,
        mut envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, crate::models::middleware::types::Error> {
        let path = options.get("path").cloned().unwrap_or_default();

        // Add or modify normalized data in the envelope
        envelope.normalized_data = Some(serde_json::json!({
            "message": "Jmix endpoint processed the request",
            "path": path,
            "original_data": envelope.original_data,
        }));

        Ok(envelope)
    }

    async fn handle_response(
        &self,
        envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Response<Self::ResBody>, crate::models::middleware::types::Error> {
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

// impl JmixEndpoint {
//     /// Handles JMIX-specific requests and responds with a placeholder JSON directly
//     pub async fn jmix_handler(
//         Path(path): Path<String>, // Path parameter extraction
//         Json(payload): Json<Value> // JSON body extraction
//     ) -> AxumJson<Value> {
//         // Process and respond with extracted data
//         AxumJson(serde_json::json!({
//             "status": "success",
//             "message": "Jmix endpoint response",
//             "path": path,
//             "payload": payload,
//         }))
//     }
// }