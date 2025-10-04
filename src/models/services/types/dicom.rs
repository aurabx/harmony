use std::collections::HashMap;
use async_trait::async_trait;
use axum::http::{Response};
use serde_json::Value;
use serde::Deserialize;
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::Envelope;
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
    type ResBody = Value;

    async fn transform_request(
        &self,
        mut envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, Error> {
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
        envelope: Envelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response<Self::ResBody>, Error> {
        // Convert the Envelope back into an HTTP response
        let body = serde_json::to_string(&envelope.normalized_data).map_err(|_| {
            Error::from("Failed to serialize DICOM response payload into JSON")
        })?;
        Response::builder()
            .status(200)
            .body(body.into())
            .map_err(|_| Error::from("Failed to construct DICOM HTTP response"))
    }
}