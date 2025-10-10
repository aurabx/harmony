use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct EchoEndpoint {}

#[async_trait]
impl ServiceType for EchoEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is non-empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.trim().is_empty())
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

        vec![RouteConfig {
            path: format!("{}/{{*wildcard}}", path_prefix), // Use {*wildcard} syntax
            methods: vec![Method::POST],
            description: Some("Handles Echo POST requests".to_string()),
        }]
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
impl ServiceHandler<Value> for EchoEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Capture subpath from request metadata inserted by dispatcher
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();

        // Add or modify the envelope's normalized data including subpath
        let full_path = envelope
            .request_details
            .metadata
            .get("full_path")
            .cloned()
            .unwrap_or_default();

        envelope.normalized_data = Some(serde_json::json!({
            "path": subpath,
            "full_path": full_path,
            "headers": envelope.request_details.headers,
            "original_data": envelope.original_data,
        }));

        Ok(envelope)
    }

    async fn transform_response(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Convention: response metadata may be present at normalized_data.response
        let nd = envelope.normalized_data.unwrap_or(serde_json::Value::Null);
        let response_meta = nd.get("response");

        // Determine status
        let status = response_meta
            .and_then(|m| m.get("status"))
            .and_then(|s| s.as_u64())
            .and_then(|code| http::StatusCode::from_u16(code as u16).ok())
            .unwrap_or(http::StatusCode::OK);

        // Build headers
        let mut builder = Response::builder().status(status);
        let mut has_content_type = false;
        if let Some(hdrs) = response_meta
            .and_then(|m| m.get("headers"))
            .and_then(|h| h.as_object())
        {
            for (k, v) in hdrs.iter() {
                if let Some(val_str) = v.as_str() {
                    if k.eq_ignore_ascii_case("content-type") {
                        has_content_type = true;
                    }
                    builder = builder.header(k.as_str(), val_str);
                }
            }
        }

        // Determine body
        if let Some(body_val) = response_meta
            .and_then(|m| m.get("body"))
            .and_then(|b| b.as_str())
        {
            // Raw body provided by middleware
            return builder
                .body(Body::from(body_val.to_string()))
                .map_err(|_| Error::from("Failed to construct Echo HTTP response"));
        }

        // Fallback to JSON serialization of normalized_data
        let body_str = serde_json::to_string(&nd)
            .map_err(|_| Error::from("Failed to serialize Echo response JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct Echo HTTP response"))
    }
}
