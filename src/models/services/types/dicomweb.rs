use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use base64::Engine;
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct DicomwebEndpoint {}

impl DicomwebEndpoint {
    /// Handle DICOMweb-specific response types with appropriate HTTP semantics
    async fn handle_dicomweb_response(
        &self,
        response_type: &str,
        nd: &Value,
    ) -> Result<Response, Error> {
        let data = nd.get("dicomweb_data");
        let metadata = nd.get("dicomweb_metadata").and_then(|v| v.as_object());
        
        match response_type {
            "qido_json" => {
                // QIDO-RS responses: application/dicom+json
                let has_results = metadata
                    .and_then(|m| m.get("has_results"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                
                let status = if has_results {
                    http::StatusCode::OK
                } else {
                    http::StatusCode::NO_CONTENT
                };
                
                let json_data = data.cloned().unwrap_or(Value::Array(vec![]));
                let body_str = serde_json::to_string(&json_data)
                    .map_err(|_| Error::from("Failed to serialize QIDO JSON"))?;
                
                Response::builder()
                    .status(status)
                    .header("content-type", "application/dicom+json")
                    .body(Body::from(body_str))
                    .map_err(|_| Error::from("Failed to construct QIDO response"))
            }
            "wado_metadata" => {
                // WADO-RS metadata responses: application/dicom+json
                let json_data = data.cloned().unwrap_or(Value::Array(vec![]));
                let body_str = serde_json::to_string(&json_data)
                    .map_err(|_| Error::from("Failed to serialize WADO metadata JSON"))?;
                
                Response::builder()
                    .status(http::StatusCode::OK)
                    .header("content-type", "application/dicom+json")
                    .body(Body::from(body_str))
                    .map_err(|_| Error::from("Failed to construct WADO metadata response"))
            }
            "wado_instance" => {
                // WADO-RS instance responses: multipart/related; type="application/dicom"
                if let Some(meta) = metadata {
                    if let (Some(boundary), Some(body_b64)) = (
                        meta.get("boundary").and_then(|v| v.as_str()),
                        meta.get("body_b64").and_then(|v| v.as_str())
                    ) {
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(body_b64)
                            .map_err(|_| Error::from("Failed to decode WADO instance body_b64"))?;
                        
                        let content_type = format!(
                            "multipart/related; type=\"application/dicom\"; boundary={}",
                            boundary
                        );
                        
                        return Response::builder()
                            .status(http::StatusCode::OK)
                            .header("content-type", content_type)
                            .body(Body::from(bytes))
                            .map_err(|_| Error::from("Failed to construct WADO instance response"));
                    }
                }
                // Fallback to error if metadata is missing
                Response::builder()
                    .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"error":"Missing instance data"}"#))
                    .map_err(|_| Error::from("Failed to construct error response"))
            }
            "wado_frames" => {
                // WADO-RS frame responses: image/jpeg, image/png, or multipart
                if let Some(meta) = metadata {
                    let content_type = meta.get("content_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("image/jpeg");
                    let is_single_frame = meta.get("is_single_frame")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    
                    if let Some(body_b64) = meta.get("body_b64").and_then(|v| v.as_str()) {
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(body_b64)
                            .map_err(|_| Error::from("Failed to decode frames body_b64"))?;
                        
                        let final_content_type = if is_single_frame {
                            content_type.to_string()
                        } else if let Some(boundary) = meta.get("boundary").and_then(|v| v.as_str()) {
                            format!("multipart/related; type=\"{}\"; boundary={}", content_type, boundary)
                        } else {
                            content_type.to_string()
                        };
                        
                        return Response::builder()
                            .status(http::StatusCode::OK)
                            .header("content-type", final_content_type)
                            .body(Body::from(bytes))
                            .map_err(|_| Error::from("Failed to construct frames response"));
                    }
                }
                // Fallback to error if metadata is missing
                Response::builder()
                    .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"error":"Missing frame data"}"#))
                    .map_err(|_| Error::from("Failed to construct error response"))
            }
            "wado_frames_error" => {
                // Handle frame decoding errors
                let error_msg = metadata
                    .and_then(|m| m.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unable to decode frames for requested instance");
                
                let error_response = serde_json::json!({
                    "error": "UnsupportedTransferSyntax",
                    "message": error_msg,
                });
                
                let body_str = serde_json::to_string(&error_response)
                    .map_err(|_| Error::from("Failed to serialize error response"))?;
                
                Response::builder()
                    .status(http::StatusCode::NOT_ACCEPTABLE)
                    .header("content-type", "application/json")
                    .body(Body::from(body_str))
                    .map_err(|_| Error::from("Failed to construct error response"))
            }
            _ => {
                // Unknown response type - serialize as JSON
                let body_str = serde_json::to_string(nd)
                    .map_err(|_| Error::from("Failed to serialize unknown response type"))?;
                
                Response::builder()
                    .status(http::StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(body_str))
                    .map_err(|_| Error::from("Failed to construct response"))
            }
        }
    }
}

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
            .unwrap_or("");
        let base = path_prefix.trim_end_matches('/');

        let routes = vec![
            // QIDO-RS: Query for studies
            RouteConfig {
                path: format!("{}/studies", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb QIDO-RS: Query for studies".to_string()),
            },
            // QIDO-RS: Query for specific study
            RouteConfig {
                path: format!("{}/studies/{{study_uid}}", base),
                methods: vec![Method::GET],
                description: Some("DICOMweb QIDO-RS: Query for specific study".to_string()),
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
        let mut set_response =
            |status: http::StatusCode,
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
            hdrs.insert(
                "access-control-allow-methods".to_string(),
                "GET, OPTIONS".to_string(),
            );
            hdrs.insert(
                "access-control-allow-headers".to_string(),
                "accept, content-type".to_string(),
            );
            set_response(http::StatusCode::OK, hdrs, None, None);
            // Skip backends for OPTIONS requests
            envelope
                .request_details
                .metadata
                .insert("skip_backends".to_string(), "true".to_string());
            return Ok(envelope);
        }

        // Check if this is a QIDO or WADO endpoint that should be processed
        let parts: Vec<&str> = subpath.split('/').filter(|s| !s.is_empty()).collect();
        let should_process = match parts.as_slice() {
            // QIDO endpoints
            ["studies"] => true,
            ["studies", _] => true,
            ["studies", _, "series"] => true,
            ["studies", _, "series", _, "instances"] => true,
            // WADO endpoints
            ["studies", _, "metadata"] => true,
            ["studies", _, "series", _, "metadata"] => true,
            ["studies", _, "series", _, "instances", _, "metadata"] => true,
            ["studies", _, "series", _, "instances", _] => true,
            ["studies", _, "series", _, "instances", _, "frames", _] => true,
            ["bulkdata", ..] => true,
            _ => false,
        };

        if should_process {
            // QIDO and WADO endpoints are implemented - allow backend processing
            // Do not set skip_backends, let the middleware and backend handle it
            return Ok(envelope);
        }

        // Only truly unimplemented endpoints return 501 Not Implemented
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

        // Skip backends for WADO endpoints that are not yet implemented
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
        
        // Check for DICOMweb-specific response types from middleware
        if let Some(response_type) = nd.get("dicomweb_response_type").and_then(|v| v.as_str()) {
            return self.handle_dicomweb_response(response_type, &nd).await;
        }
        
        // Fallback to legacy response handling for backward compatibility
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{RequestEnvelope, RequestDetails};
    use axum::body::to_bytes;
    use std::collections::HashMap;
    
    #[tokio::test]
    async fn test_qido_response_with_results() {
        let endpoint = DicomwebEndpoint {};
        
        // Create mock normalized data for QIDO response with results
        let qido_data = serde_json::json!([
            {
                "00100020": {"vr": "LO", "Value": ["12345"]},
                "00100010": {"vr": "PN", "Value": ["Doe^John"]}
            }
        ]);
        
        let mut metadata = serde_json::Map::new();
        metadata.insert("has_results".to_string(), Value::Bool(true));
        
        let normalized_data = serde_json::json!({
            "dicomweb_response_type": "qido_json",
            "dicomweb_data": qido_data,
            "dicomweb_metadata": metadata
        });
        
        let envelope = RequestEnvelope {
            request_details: RequestDetails {
                method: "GET".to_string(),
                uri: "/dicomweb/studies".to_string(),
                headers: HashMap::new(),
                cookies: HashMap::new(),
                query_params: HashMap::new(),
                cache_status: None,
                metadata: HashMap::new(),
            },
            original_data: vec![],
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };
        
        let response = endpoint.transform_response(envelope, &HashMap::new()).await;
        assert!(response.is_ok());
        
        let resp = response.unwrap();
        
        // Verify status code is 200 OK
        assert_eq!(resp.status(), http::StatusCode::OK);
        
        // Verify content-type header
        let content_type = resp.headers().get("content-type");
        assert!(content_type.is_some());
        assert_eq!(content_type.unwrap(), "application/dicom+json");
    }
    
    #[tokio::test]
    async fn test_qido_response_empty_results() {
        let endpoint = DicomwebEndpoint {};
        
        let mut metadata = serde_json::Map::new();
        metadata.insert("has_results".to_string(), Value::Bool(false));
        
        let normalized_data = serde_json::json!({
            "dicomweb_response_type": "qido_json",
            "dicomweb_data": [],
            "dicomweb_metadata": metadata
        });
        
        let envelope = RequestEnvelope {
            request_details: RequestDetails {
                method: "GET".to_string(),
                uri: "/dicomweb/studies".to_string(),
                headers: HashMap::new(),
                cookies: HashMap::new(),
                query_params: HashMap::new(),
                cache_status: None,
                metadata: HashMap::new(),
            },
            original_data: vec![],
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };
        
        let response = endpoint.transform_response(envelope, &HashMap::new()).await;
        assert!(response.is_ok());
        
        let resp = response.unwrap();
        
        // Verify status code is 204 No Content
        assert_eq!(resp.status(), http::StatusCode::NO_CONTENT);
        
        // Verify content-type header
        let content_type = resp.headers().get("content-type");
        assert!(content_type.is_some());
        assert_eq!(content_type.unwrap(), "application/dicom+json");
    }
    
    #[tokio::test]
    async fn test_wado_frames_single_image() {
        let endpoint = DicomwebEndpoint {};
        
        let mut metadata = serde_json::Map::new();
        metadata.insert("content_type".to_string(), Value::String("image/jpeg".to_string()));
        metadata.insert("body_b64".to_string(), Value::String("SGVsbG8gV29ybGQ=".to_string())); // "Hello World" in base64
        metadata.insert("is_single_frame".to_string(), Value::Bool(true));
        
        let normalized_data = serde_json::json!({
            "dicomweb_response_type": "wado_frames",
            "dicomweb_data": serde_json::Value::Null,
            "dicomweb_metadata": metadata
        });
        
        let envelope = RequestEnvelope {
            request_details: RequestDetails {
                method: "GET".to_string(),
                uri: "/dicomweb/studies/1.2.3/series/4.5.6/instances/7.8.9/frames/1".to_string(),
                headers: HashMap::new(),
                cookies: HashMap::new(),
                query_params: HashMap::new(),
                cache_status: None,
                metadata: HashMap::new(),
            },
            original_data: vec![],
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };
        
        let response = endpoint.transform_response(envelope, &HashMap::new()).await;
        assert!(response.is_ok());
        
        let resp = response.unwrap();
        
        // Verify status code is 200 OK
        assert_eq!(resp.status(), http::StatusCode::OK);
        
        // Verify content-type header is image/jpeg
        let content_type = resp.headers().get("content-type");
        assert!(content_type.is_some());
        assert_eq!(content_type.unwrap(), "image/jpeg");
        
        // Verify body contains decoded base64 data
        let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body_bytes.as_ref(), b"Hello World");
    }
    
    #[tokio::test]
    async fn test_wado_frames_error() {
        let endpoint = DicomwebEndpoint {};
        
        let mut metadata = serde_json::Map::new();
        metadata.insert("error".to_string(), Value::String("UnsupportedTransferSyntax".to_string()));
        metadata.insert("message".to_string(), Value::String("Unable to decode frames for requested instance".to_string()));
        
        let normalized_data = serde_json::json!({
            "dicomweb_response_type": "wado_frames_error",
            "dicomweb_data": serde_json::Value::Null,
            "dicomweb_metadata": metadata
        });
        
        let envelope = RequestEnvelope {
            request_details: RequestDetails {
                method: "GET".to_string(),
                uri: "/dicomweb/studies/1.2.3/series/4.5.6/instances/7.8.9/frames/1".to_string(),
                headers: HashMap::new(),
                cookies: HashMap::new(),
                query_params: HashMap::new(),
                cache_status: None,
                metadata: HashMap::new(),
            },
            original_data: vec![],
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };
        
        let response = endpoint.transform_response(envelope, &HashMap::new()).await;
        assert!(response.is_ok());
        
        let resp = response.unwrap();
        
        // Verify status code is 406 Not Acceptable
        assert_eq!(resp.status(), http::StatusCode::NOT_ACCEPTABLE);
        
        // Verify content-type header is application/json
        let content_type = resp.headers().get("content-type");
        assert!(content_type.is_some());
        assert_eq!(content_type.unwrap(), "application/json");
        
        // Verify error response body
        let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&body_str).unwrap();
        assert_eq!(json["error"], "UnsupportedTransferSyntax");
        assert_eq!(json["message"], "Unable to decode frames for requested instance");
    }
}
