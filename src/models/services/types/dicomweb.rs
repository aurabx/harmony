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
use base64::Engine;

#[derive(Debug, Deserialize)]
pub struct DicomwebEndpoint {}

#[async_trait]
impl ServiceType for DicomwebEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Ensure 'path_prefix' exists and is non-empty
        if options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicomweb".to_string(),
                reason: "DICOMweb endpoint requires a non-empty 'path_prefix'".to_string(),
            });
        }
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // DICOMweb exposes specific QIDO-RS and WADO-RS routes
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("/dicomweb");
        let base = path_prefix.trim_end_matches('/');

        let routes = vec![
            // QIDO-RS: Query for studies
            RouteConfig {
                path: format!("{}/studies", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb QIDO-RS: Query for studies".to_string()),
            },
            // QIDO-RS: Query for series within a study
            RouteConfig {
                path: format!("{}/studies/{{study_uid}}/series", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb QIDO-RS: Query for series".to_string()),
            },
            // QIDO-RS: Query for instances within a series
            RouteConfig {
                path: format!("{}/studies/{{study_uid}}/series/{{series_uid}}/instances", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb QIDO-RS: Query for instances".to_string()),
            },
            // WADO-RS: Retrieve study metadata
            RouteConfig {
                path: format!("{}/studies/{{study_uid}}/metadata", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb WADO-RS: Retrieve study metadata".to_string()),
            },
            // WADO-RS: Retrieve series metadata
            RouteConfig {
                path: format!("{}/studies/{{study_uid}}/series/{{series_uid}}/metadata", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb WADO-RS: Retrieve series metadata".to_string()),
            },
            // WADO-RS: Retrieve instance metadata
            RouteConfig {
                path: format!("{}/studies/{{study_uid}}/series/{{series_uid}}/instances/{{instance_uid}}/metadata", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb WADO-RS: Retrieve instance metadata".to_string()),
            },
            // WADO-RS: Retrieve instance (DICOM object)
            RouteConfig {
                path: format!("{}/studies/{{study_uid}}/series/{{series_uid}}/instances/{{instance_uid}}", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb WADO-RS: Retrieve instance".to_string()),
            },
            // WADO-RS: Retrieve rendered image frames
            RouteConfig {
                path: format!("{}/studies/{{study_uid}}/series/{{series_uid}}/instances/{{instance_uid}}/frames/{{frame_numbers}}", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb WADO-RS: Retrieve rendered frames".to_string()),
            },
            // WADO-RS: Bulk data retrieval
            RouteConfig {
                path: format!("{}/bulkdata/{{*bulk_data_uri}}", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb WADO-RS: Bulk data retrieval".to_string()),
            },
        ];

        // Add OPTIONS for CORS support on all routes
        routes
            .into_iter()
            .map(|mut rc| {
                if !rc.methods.contains(&http::Method::OPTIONS) {
                    rc.methods.push(http::Method::OPTIONS);
                }
                rc
            })
            .collect()
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
            "DicomwebEndpoint only supports Protocol::Http envelope building",
        ))
    }
}

#[async_trait]
impl ServiceHandler<Value> for DicomwebEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        let method = envelope.request_details.method.to_uppercase();
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();

        // Helper: set response meta into normalized_data
        let mut set_response = |status: http::StatusCode,
                                hdrs: HashMap<String, String>,
                                body_str: Option<String>,
                                json_obj: Option<serde_json::Value>| {
            let mut resp = serde_json::Map::new();
            resp.insert("status".to_string(), serde_json::json!(status.as_u16()));
            if !hdrs.is_empty() {
                resp.insert("headers".to_string(), serde_json::json!(hdrs));
            }
            if let Some(s) = body_str {
                resp.insert("body".to_string(), serde_json::json!(s));
            }
            if let Some(j) = json_obj {
                resp.insert("json".to_string(), j);
            }
            envelope.normalized_data = Some(serde_json::json!({
                "response": serde_json::Value::Object(resp)
            }));
        };

        // Handle OPTIONS requests for CORS
        if method == "OPTIONS" {
            let mut hdrs = HashMap::new();
            hdrs.insert("access-control-allow-origin".to_string(), "*".to_string());
            hdrs.insert("access-control-allow-methods".to_string(), "GET, OPTIONS".to_string());
            hdrs.insert("access-control-allow-headers".to_string(), "accept, content-type".to_string());
            set_response(http::StatusCode::OK, hdrs, None, None);
            // Skip backends for OPTIONS requests
            envelope
                .request_details
                .metadata
                .insert("skip_backends".to_string(), "true".to_string());
            return Ok(envelope);
        }

        // For now, all DICOMweb endpoints return 501 Not Implemented
        // This is the skeleton implementation as requested
        let mut hdrs = HashMap::new();
        hdrs.insert("content-type".to_string(), "application/json".to_string());
        
        let error_response = serde_json::json!({
            "error": "Not implemented",
            "message": format!("DICOMweb endpoint {} {} is not yet implemented", method, subpath),
            "path": subpath,
            "method": method
        });

        set_response(
            http::StatusCode::NOT_IMPLEMENTED,
            hdrs,
            None,
            Some(error_response),
        );

        // Skip backends for now since this is just the skeleton
        envelope
            .request_details
            .metadata
            .insert("skip_backends".to_string(), "true".to_string());

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

        // body as explicit string
        if let Some(body_str) = response_meta
            .and_then(|m| m.get("body"))
            .and_then(|b| b.as_str())
        {
            return builder
                .body(Body::from(body_str.to_string()))
                .map_err(|_| Error::from("Failed to construct DICOMweb HTTP response"));
        }

        // body as base64 (binary)
        if let Some(body_b64) = response_meta
            .and_then(|m| m.get("body_b64"))
            .and_then(|b| b.as_str())
        {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(body_b64)
                .map_err(|_| Error::from("Failed to decode body_b64"))?;
            return builder
                .body(Body::from(bytes))
                .map_err(|_| Error::from("Failed to construct DICOMweb HTTP response"));
        }

        // body as JSON object under response.json
        if let Some(json_val) = response_meta.and_then(|m| m.get("json")) {
            let body_str = serde_json::to_string(json_val)
                .map_err(|_| Error::from("Failed to serialize DICOMweb response JSON"))?;
            if !has_content_type {
                builder = builder.header("content-type", "application/json");
            }
            return builder
                .body(Body::from(body_str))
                .map_err(|_| Error::from("Failed to construct DICOMweb HTTP response"));
        }

        // default: serialize entire normalized_data
        let body_str = serde_json::to_string(&nd)
            .map_err(|_| Error::from("Failed to serialize DICOMweb response payload into JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct DICOMweb HTTP response"))
    }
}