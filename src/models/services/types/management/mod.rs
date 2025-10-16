pub(crate) use self::config::ManagementConfig;
use self::info::handle_info;
use self::pipelines::handle_pipelines;
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::pipelines::config::Pipeline;
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, extract::State, response::Response};
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

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
        ]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        use crate::models::envelope::envelope::RequestDetails;
        use std::collections::HashMap;

        // Convert protocol context to request details
        let mut headers = HashMap::new();
        let mut cookies = HashMap::new();
        let mut query_params = HashMap::new();
        let metadata = ctx.meta;

        // Extract data from attrs if it's an object
        if let Some(attrs_obj) = ctx.attrs.as_object() {
            // Extract headers
            if let Some(headers_val) = attrs_obj.get("headers").and_then(|v| v.as_object()) {
                for (k, v) in headers_val {
                    if let Some(v_str) = v.as_str() {
                        headers.insert(k.clone(), v_str.to_string());
                    }
                }
            }

            // Extract cookies
            if let Some(cookies_val) = attrs_obj.get("cookies").and_then(|v| v.as_object()) {
                for (k, v) in cookies_val {
                    if let Some(v_str) = v.as_str() {
                        cookies.insert(k.clone(), v_str.to_string());
                    }
                }
            }

            // Extract query params
            if let Some(query_val) = attrs_obj.get("query_params").and_then(|v| v.as_object()) {
                for (k, v) in query_val {
                    if let Some(v_array) = v.as_array() {
                        let strings: Vec<String> = v_array
                            .iter()
                            .filter_map(|item| item.as_str().map(|s| s.to_string()))
                            .collect();
                        query_params.insert(k.clone(), strings);
                    }
                }
            }
        }

        let request_details = RequestDetails {
            method: ctx
                .attrs
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET")
                .to_string(),
            uri: ctx
                .attrs
                .get("uri")
                .and_then(|v| v.as_str())
                .unwrap_or("/")
                .to_string(),
            headers,
            cookies,
            query_params,
            cache_status: ctx
                .attrs
                .get("cache_status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            metadata,
        };

        Ok(RequestEnvelope::new(request_details, ctx.payload))
    }
}

#[async_trait]
impl ServiceHandler<Value> for ManagementEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // No transformation needed for management endpoints
        Ok(envelope)
    }

    async fn transform_response(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
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

        let response_value = match clean_path {
            p if p == "info" || p == format!("{}/info", base_path) => {
                let info = handle_info().await;
                serde_json::to_value(info.0)
                    .map_err(|_| Error::from("Failed to serialize info response"))?
            }
            p if p == "pipelines" || p == format!("{}/pipelines", base_path) => {
                // Get pipelines from service options if available
                let pipelines = options
                    .get("pipelines")
                    .and_then(|v| v.as_object())
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| {
                                serde_json::from_value::<Pipeline>(v.clone())
                                    .ok()
                                    .map(|p| (k.clone(), p))
                            })
                            .collect::<HashMap<String, Pipeline>>()
                    })
                    .unwrap_or_default();

                let pipelines_response = handle_pipelines(State(Arc::new(pipelines))).await;
                serde_json::to_value(pipelines_response.0)
                    .map_err(|_| Error::from("Failed to serialize pipelines response"))?
            }
            p if p == "routes" || p == format!("{}/routes", base_path) => {
                // Use global config access since we need full config for routes analysis
                let config = crate::globals::get_config();
                let routes_response = if let Some(config_arc) = config {
                    self::routes::get_routes_info(&config_arc)
                } else {
                    self::routes::RoutesResponse { routes: vec![] }
                };
                serde_json::to_value(routes_response)
                    .map_err(|_| Error::from("Failed to serialize routes response"))?
            }
            _ => serde_json::json!({"error": "Not found"}),
        };

        let body = serde_json::to_string(&response_value)
            .map_err(|_| Error::from("Failed to serialize management response"))?;

        Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .map_err(|_| Error::from("Failed to construct management response"))
    }
}
