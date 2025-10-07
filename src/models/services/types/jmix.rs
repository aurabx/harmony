use std::collections::HashMap;
use async_trait::async_trait;
use axum::{response::Response, body::Body};
use serde_json::Value;
use serde::Deserialize;
use crate::models::envelope::envelope::RequestEnvelope;
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

    async fn build_request_envelope(
        &self,
        req: &mut axum::extract::Request,
        options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error> {
        // Delegate to HttpEndpoint builder for consistent HTTP parsing
        let http = crate::models::services::types::http::HttpEndpoint {};
        http.build_request_envelope(req, options).await
    }
}

#[async_trait]
impl ServiceHandler<Value> for JmixEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
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
                .map_err(|_| Error::from("Failed to construct JMIX HTTP response"));
        }

        let body_str = serde_json::to_string(&nd).map_err(|_| Error::from("Failed to serialize Jmix response payload into JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct JMIX HTTP response"))
    }
}
