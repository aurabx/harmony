use std::collections::HashMap;
use async_trait::async_trait;
use axum::{response::Response, body::Body};
use serde_json::Value;
use serde::Deserialize;
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::services::services::{ServiceType, ServiceHandler};

use http::Method;
use crate::utils::Error;
use crate::router::route_config::RouteConfig;

#[derive(Debug, Deserialize)]
pub struct DicomEndpoint {
    pub aet: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
}

impl ServiceType for DicomEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        if options
            .get("aet")
            .and_then(|v| v.as_str())
            .map_or(true, |s| s.trim().is_empty())
        {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Missing or empty 'aet' (Application Entity Title)".to_string(),
            });
        }

        if options
            .get("host")
            .and_then(|v| v.as_str())
            .map_or(true, |s| s.trim().is_empty())
        {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Missing or empty 'host' (DICOM server address)".to_string(),
            });
        }

        if options
            .get("port")
            .and_then(|v| v.as_u64())
            .map_or(true, |p| !(1024..=65535).contains(&p))
        {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Invalid 'port' (Allowed range: 1024-65535)".to_string(),
            });
        }

        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("/dicom");

        vec![
            RouteConfig {
                path: format!("{}/store", path_prefix),
                methods: vec![Method::POST],
                description: Some("Handles DICOM object storage requests".to_string()),
            },
            RouteConfig {
                path: format!("{}/query", path_prefix),
                methods: vec![Method::GET],
                description: Some("Handles DICOM query requests".to_string()),
            },
        ]
    }
}

#[async_trait]
impl ServiceHandler<Value> for DicomEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        let aet = options
            .get("aet")
            .and_then(|v| v.as_str())
            .unwrap_or("default-aet");

        // Add or modify normalized data in the envelope
        envelope.normalized_data = Some(serde_json::json!({
            "message": "DICOM request processed",
            "aet": aet,
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
                .map_err(|_| Error::from("Failed to construct DICOM HTTP response"));
        }

        let body_str = serde_json::to_string(&nd).map_err(|_| Error::from("Failed to serialize DICOM response payload into JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct DICOM HTTP response"))
    }
}