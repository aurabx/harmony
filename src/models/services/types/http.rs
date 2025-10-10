use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct HttpEndpoint {}

#[async_trait]
impl ServiceType for HttpEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is not empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.trim().is_empty())
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

        vec![RouteConfig {
            path: format!("{}/{{*wildcard}}", path_prefix),
            methods: vec![
                http::Method::GET,
                http::Method::POST,
                http::Method::PUT,
                http::Method::DELETE,
            ],
            description: Some("Handles GET/POST/PUT/DELETE for HttpEndpoint".to_string()),
        }]
    }

    // noinspection DuplicatedCode
    // Protocol-agnostic builder (HTTP variant)
    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        use crate::models::envelope::envelope::{RequestDetails, RequestEnvelope};
        use crate::utils::Error;
        use std::collections::HashMap as Map;

        if ctx.protocol != crate::models::protocol::Protocol::Http {
            return Err(Error::from(
                "HttpEndpoint only supports Protocol::Http in build_protocol_envelope",
            ));
        }

        let attrs = ctx
            .attrs
            .as_object()
            .ok_or_else(|| Error::from("invalid attrs for HTTP"))?;
        let headers_map: Map<String, String> = attrs
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let cookies_map: Map<String, String> = attrs
            .get("cookies")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let query_params: Map<String, Vec<String>> = attrs
            .get("query_params")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let vec = v
                            .as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>();
                        (k.clone(), vec)
                    })
                    .collect::<Map<String, Vec<String>>>()
            })
            .unwrap_or_default();
        let cache_status = attrs
            .get("cache_status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut metadata: Map<String, String> = Map::new();
        // pass through HTTP-derived meta (path, full_path) from ctx.meta
        if let Some(path) = ctx.meta.get("path") {
            metadata.insert("path".into(), path.clone());
        }
        if let Some(full) = ctx.meta.get("full_path") {
            metadata.insert("full_path".into(), full.clone());
        }
        if let Some(proto) = ctx.meta.get("protocol") {
            metadata.insert("protocol".into(), proto.clone());
        }

        let method = attrs
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let uri = attrs
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let request_details = RequestDetails {
            method,
            uri,
            headers: headers_map,
            cookies: cookies_map,
            query_params,
            cache_status,
            metadata,
        };

        // Try to parse payload as JSON for normalized_data
        let normalized_data = serde_json::from_slice(&ctx.payload).ok();
        
        Ok(RequestEnvelope {
            request_details,
            original_data: ctx.payload,
            normalized_data,
            normalized_snapshot: None,
        })
    }
}

#[async_trait]
impl ServiceHandler<Value> for HttpEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Populate normalized data with real request context
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();
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
        let nd = envelope.normalized_data.unwrap_or(serde_json::Value::Null);
        let response_meta = nd.get("response");

        let status = response_meta
            .and_then(|m| m.get("status"))
            .and_then(|s| s.as_u64())
            .and_then(|code| http::StatusCode::from_u16(code as u16).ok())
            .unwrap_or(http::StatusCode::OK);

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

        if let Some(body_str) = response_meta
            .and_then(|m| m.get("body"))
            .and_then(|b| b.as_str())
        {
            return builder
                .body(Body::from(body_str.to_string()))
                .map_err(|_| Error::from("Failed to construct HTTP response"));
        }

        let body_str = serde_json::to_string(&nd)
            .map_err(|_| Error::from("Failed to serialize HTTP response JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct HTTP response"))
    }
}
