pub(crate) use self::config::ManagementConfig;
use self::info::handle_info;
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

pub mod authorize;
pub mod config;
pub mod info;
pub mod pipelines;
pub mod routes;

#[derive(Debug, Deserialize)]
pub struct ManagementEndpoint {}

#[async_trait]
impl ServiceType for ManagementEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate the management configuration if present
        if let Some(config) = options.get("config").and_then(|v| v.as_object()) {
            let config: ManagementConfig = serde_json::from_value(Value::Object(config.clone()))
                .map_err(|_| ConfigError::InvalidEndpoint {
                    name: "management".to_string(),
                    reason: "Invalid management configuration".to_string(),
                })?;
            config
                .validate()
                .map_err(|e| ConfigError::InvalidEndpoint {
                    name: "management".to_string(),
                    reason: e,
                })?;
        }
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Get base_path from config or use default
        let base_path = options
            .get("config")
            .and_then(|v| v.as_object())
            .and_then(|c| c.get("base_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("admin");

        vec![
            RouteConfig {
                path: format!("/{}/info", base_path),
                methods: vec![Method::GET],
                description: Some("Get system information".to_string()),
            },
            RouteConfig {
                path: format!("/{}/pipelines", base_path),
                methods: vec![Method::GET],
                description: Some("List all configured pipelines".to_string()),
            },
            RouteConfig {
                path: format!("/{}/routes", base_path),
                methods: vec![Method::GET],
                description: Some("List all configured routes".to_string()),
            },
            RouteConfig {
                path: format!("/{}/authorize", base_path),
                methods: vec![Method::POST],
                description: Some("Authorize gateway with Runbeam Cloud".to_string()),
            },
        ]
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
impl ServiceHandler<Value> for ManagementEndpoint {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // No transformation needed for management endpoints
        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // Management endpoints generate responses directly (no backend call)
        // Extract the path from metadata to determine the handler
        let path = envelope
            .request_details
            .metadata
            .get("path")
            .map(|s| s.as_str())
            .unwrap_or("");

        // Get base_path from config to match against
        let base_path = options
            .get("config")
            .and_then(|v| v.as_object())
            .and_then(|c| c.get("base_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("admin");

        // Remove leading slash and match the specific endpoint
        let clean_path = path.trim_start_matches('/');

        let (response_value, status_code) = match clean_path {
            p if p == "info" || p == format!("{}/info", base_path) => {
                let info = handle_info().await;
                let value = serde_json::to_value(info.0)
                    .map_err(|_| Error::from("Failed to serialize info response"))?;
                (value, 200)
            }
            p if p == "pipelines" || p == format!("{}/pipelines", base_path) => {
                // Use global config access to get pipelines
                let config = crate::globals::get_config();
                let pipelines_response = if let Some(config_arc) = config {
                    self::pipelines::get_pipelines_info(&config_arc.pipelines)
                } else {
                    self::pipelines::PipelinesResponse { pipelines: vec![] }
                };
                let value = serde_json::to_value(pipelines_response)
                    .map_err(|_| Error::from("Failed to serialize pipelines response"))?;
                (value, 200)
            }
            p if p == "routes" || p == format!("{}/routes", base_path) => {
                // Use global config access since we need full config for routes analysis
                let config = crate::globals::get_config();
                let routes_response = if let Some(config_arc) = config {
                    self::routes::get_routes_info(&config_arc)
                } else {
                    self::routes::RoutesResponse { routes: vec![] }
                };
                let value = serde_json::to_value(routes_response)
                    .map_err(|_| Error::from("Failed to serialize routes response"))?;
                (value, 200)
            }
            p if p == "authorize" || p == format!("{}/authorize", base_path) => {
                // Handle gateway authorization
                let auth_header = envelope.request_details.headers.get("authorization").map(|s| s.as_str());
                match self::authorize::handle_authorize(auth_header, &envelope.original_data).await {
                    Ok(value) => (value, 201),
                    Err((status, message)) => {
                        let error_json = serde_json::json!({
                            "error": http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR).canonical_reason().unwrap_or("Error"),
                            "message": message
                        });
                        (error_json, status)
                    }
                }
            }
            _ => (serde_json::json!({"error": "Not found"}), 404),
        };

        let body = serde_json::to_vec(&response_value)
            .map_err(|_| Error::from("Failed to serialize management response"))?;

        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status_code,
            headers,
            body,
            None,
        );

        response_envelope.normalized_data = Some(response_value);

        Ok(response_envelope)
    }

    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));
        envelope
            .response_details
            .metadata
            .insert("service".to_string(), "management".to_string());
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
            let body_bytes = serde_json::to_vec(&normalized)
                .map_err(|_| Error::from("Failed to serialize management response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct management response"))
    }
}
