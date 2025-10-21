use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use harmony_transform::{JoltTransformEngine, TransformConfig};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct JoltTransformMiddlewareConfig {
    /// Path to the JOLT spec JSON file
    pub spec_path: String,
    /// Apply transform on which direction: "left", "right", or "both" (default)
    #[serde(default = "default_apply")]
    pub apply: String,
    /// Whether to fail the request on transform errors
    #[serde(default = "default_fail_on_error")]
    pub fail_on_error: bool,
}

fn default_apply() -> String {
    "both".to_string()
}

fn default_fail_on_error() -> bool {
    true
}

impl From<JoltTransformMiddlewareConfig> for TransformConfig {
    fn from(config: JoltTransformMiddlewareConfig) -> Self {
        TransformConfig {
            spec_path: config.spec_path,
            apply: config.apply,
            fail_on_error: config.fail_on_error,
        }
    }
}

pub struct JoltTransformMiddleware {
    engine: JoltTransformEngine,
}

impl JoltTransformMiddleware {
    pub fn new(config: JoltTransformMiddlewareConfig) -> Result<Self, String> {
        let transform_config: TransformConfig = config.into();
        let engine = JoltTransformEngine::new(transform_config)
            .map_err(|e| format!("Failed to create JOLT transform engine: {}", e))?;

        tracing::info!("JOLT transform middleware initialized");
        Ok(Self { engine })
    }
}

#[async_trait]
impl Middleware for JoltTransformMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        if !self.engine.should_apply_left() {
            return Ok(envelope);
        }

        // Store snapshot before transformation if not already present
        if envelope.normalized_snapshot.is_none() {
            envelope.normalized_snapshot = envelope.normalized_data.clone();
        }

        // Apply transform to normalized_data
        if let Some(ref normalized_data) = envelope.normalized_data.clone() {
            match self.engine.transform(normalized_data.clone()) {
                Ok(transformed) => {
                    envelope.normalized_data = Some(transformed);
                    envelope.original_data = envelope
                        .normalized_data
                        .clone()
                        .unwrap_or(serde_json::Value::Null);
                    tracing::debug!("Applied JOLT transform on left side");
                }
                Err(e) => {
                    let error_msg = format!("JOLT transform failed on left side: {}", e);
                    if self.engine.should_fail_on_error() {
                        tracing::error!("{}", error_msg);
                        return Err(Error::from(error_msg));
                    } else {
                        tracing::warn!("{}, continuing with original data", error_msg);
                    }
                }
            }
        }

        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        if !self.engine.should_apply_right() {
            return Ok(envelope);
        }

        // Store snapshot before transformation if not already present
        if envelope.normalized_snapshot.is_none() {
            envelope.normalized_snapshot = envelope.normalized_data.clone();
        }

        // Apply transform to normalized_data (response data)
        if let Some(ref normalized_data) = envelope.normalized_data.clone() {
            match self.engine.transform(normalized_data.clone()) {
                Ok(transformed) => {
                    envelope.normalized_data = Some(transformed);
                    envelope.original_data = envelope
                        .normalized_data
                        .clone()
                        .unwrap_or(serde_json::Value::Null);
                    tracing::debug!("Applied JOLT transform on response (right side)");
                }
                Err(e) => {
                    let error_msg = format!("JOLT transform failed on response: {}", e);
                    if self.engine.should_fail_on_error() {
                        tracing::error!("{}", error_msg);
                        return Err(Error::from(error_msg));
                    } else {
                        tracing::warn!("{}, continuing with original data", error_msg);
                    }
                }
            }
        }

        Ok(envelope)
    }
}

/// Parse configuration from HashMap for middleware registry
pub fn parse_config(
    options: &HashMap<String, Value>,
) -> Result<JoltTransformMiddlewareConfig, String> {
    let spec_path = options
        .get("spec_path")
        .and_then(|v| v.as_str())
        .ok_or("Missing required 'spec_path' in transform middleware config")?
        .to_string();

    let apply = options
        .get("apply")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(default_apply);

    let fail_on_error = options
        .get("fail_on_error")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(default_fail_on_error);

    Ok(JoltTransformMiddlewareConfig {
        spec_path,
        apply,
        fail_on_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{RequestDetails, RequestEnvelopeBuilder, ResponseDetails, ResponseEnvelope};
    use serde_json::json;
    use std::fs;
    use tempfile::NamedTempFile;

    fn create_test_envelope(data: Value) -> RequestEnvelope<Value> {
        RequestEnvelopeBuilder::new()
            .method("POST")
            .uri("/test")
            .original_data(data.clone())
            .normalized_data(Some(data))
            .build()
            .unwrap()
    }

    fn request_to_response(req: RequestEnvelope<Value>) -> ResponseEnvelope<Value> {
        ResponseEnvelope {
            request_details: req.request_details,
            response_details: ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            original_data: req.original_data,
            normalized_data: req.normalized_data,
            normalized_snapshot: req.normalized_snapshot,
        }
    }

    #[tokio::test]
    async fn test_jolt_transform_middleware_left() {
        // Create a temporary JOLT spec file
        let spec = json!([{
            "operation": "shift",
            "spec": {
                "name": "data.name",
                "account": "data.account"
            }
        }]);

        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, serde_json::to_string_pretty(&spec).unwrap()).unwrap();

        let config = JoltTransformMiddlewareConfig {
            spec_path: temp_file.path().to_string_lossy().to_string(),
            apply: "left".to_string(),
            fail_on_error: true,
        };

        let middleware = JoltTransformMiddleware::new(config).unwrap();

        let input = json!({
            "id": 1,
            "name": "John Smith",
            "account": {
                "id": 1000,
                "type": "Checking"
            }
        });

        let envelope = create_test_envelope(input.clone());
        let result = middleware.left(envelope).await.unwrap();

        let expected = json!({
            "data": {
                "name": "John Smith",
                "account": {
                    "id": 1000,
                    "type": "Checking"
                }
            }
        });

        assert_eq!(result.normalized_data, Some(expected));
        assert_eq!(result.normalized_snapshot, Some(input));
    }

    #[tokio::test]
    async fn test_jolt_transform_middleware_right_only() {
        // Create a simple identity transform
        let spec = json!([{
            "operation": "shift",
            "spec": {
                "*": "&"
            }
        }]);

        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, serde_json::to_string_pretty(&spec).unwrap()).unwrap();

        let config = JoltTransformMiddlewareConfig {
            spec_path: temp_file.path().to_string_lossy().to_string(),
            apply: "right".to_string(),
            fail_on_error: true,
        };

        let middleware = JoltTransformMiddleware::new(config).unwrap();

        let input = json!({"test": "value"});
        let envelope = create_test_envelope(input.clone());

        // Should be unchanged on left
        let left_result = middleware.left(envelope).await.unwrap();
        assert_eq!(left_result.normalized_data, Some(input.clone()));
        assert_eq!(left_result.normalized_snapshot, None); // No snapshot created on left when apply=right

        // Convert to ResponseEnvelope for right side
        let response_envelope = request_to_response(left_result);

        // Should apply transform on right
        let right_result = middleware.right(response_envelope).await.unwrap();
        assert_eq!(right_result.normalized_data, Some(input.clone())); // Identity transform
        assert_eq!(right_result.normalized_snapshot, Some(input)); // Snapshot created on right
    }

    #[tokio::test]
    async fn test_jolt_transform_middleware_apply_both() {
        // Identity transform applied on both sides
        let spec = json!([{
            "operation": "shift",
            "spec": {
                "*": "&"
            }
        }]);
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, serde_json::to_string_pretty(&spec).unwrap()).unwrap();

        let config = JoltTransformMiddlewareConfig {
            spec_path: temp_file.path().to_string_lossy().to_string(),
            apply: "both".to_string(),
            fail_on_error: true,
        };
        let middleware = JoltTransformMiddleware::new(config).unwrap();

        let input = json!({"k": "v"});
        let env = create_test_envelope(input.clone());

        let left_res = middleware.left(env).await.unwrap();
        assert_eq!(left_res.normalized_data, Some(input.clone()));
        assert_eq!(left_res.normalized_snapshot, Some(input.clone()));

        // Convert to ResponseEnvelope for right side
        let response_envelope = request_to_response(left_res);

        let right_res = middleware.right(response_envelope).await.unwrap();
        assert_eq!(right_res.normalized_data, Some(input.clone()));
        // snapshot should remain as first snapshot
        assert_eq!(right_res.normalized_snapshot, Some(input));
    }

    #[test]
    fn test_parse_config() {
        let mut options = HashMap::new();
        options.insert("spec_path".to_string(), json!("/path/to/spec.json"));
        options.insert("apply".to_string(), json!("both"));
        options.insert("fail_on_error".to_string(), json!(false));

        let config = parse_config(&options).unwrap();
        assert_eq!(config.spec_path, "/path/to/spec.json");
        assert_eq!(config.apply, "both");
        assert!(!config.fail_on_error);
    }

    #[test]
    fn test_parse_config_missing_spec_path() {
        let options = HashMap::new();
        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required 'spec_path'"));
    }

    #[tokio::test]
    async fn test_middleware_with_real_fhir_to_dicom_params_left() {
        // Build envelope resembling FHIR endpoint normalized_data
        let mut env = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/fhir/ImagingStudy?patient=PID156695")
            .original_data(serde_json::json!({}))
            .normalized_data(Some(serde_json::json!({
                "full_path": "/fhir/ImagingStudy?patient=PID156695",
                "path": "ImagingStudy",
                "headers": {},
                "original_data": {}
            })))
            .build()
            .unwrap();

        // Use real spec file
        let spec_path = format!(
            "{}/examples/fhir-to-dicom/transforms/fhir_to_dicom_params.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let cfg = JoltTransformMiddlewareConfig {
            spec_path,
            apply: "left".into(),
            fail_on_error: true,
        };
        let mw = JoltTransformMiddleware::new(cfg).unwrap();

        env = mw.left(env).await.unwrap();
        // Should have an object; presence of dimse_identifier is expected by current spec
        let out = env.normalized_data.unwrap();
        assert!(out.is_object());
        assert!(out.get("dimse_identifier").is_some());
    }

    #[tokio::test]
    async fn test_middleware_with_real_dicom_to_imagingstudy_right() {
        use crate::models::envelope::envelope::ResponseDetails;
        use serde_json::json;

        // Start with a DICOM find-style payload (as produced by mock_dicom)
        let request_details = RequestDetails {
            method: "GET".into(),
            uri: "/fhir/ImagingStudy?patient=PID156695".into(),
            headers: Default::default(),
            cookies: Default::default(),
            query_params: Default::default(),
            cache_status: None,
            metadata: Default::default(),
        };
        let input = json!({
            "operation": "find",
            "success": true,
            "matches": [
                {
                    "0020000D": {"vr": "UI", "Value": ["1.2.3"]},
                    "00100020": {"vr": "LO", "Value": ["PID156695"]},
                    "00100010": {"vr": "PN", "Value": [{"Alphabetic": "Doe^John"}]},
                    "00080020": {"vr": "DA", "Value": ["20241015"]},
                    "00080030": {"vr": "TM", "Value": ["120000"]},
                    "00081030": {"vr": "LO", "Value": ["Mock CT Study"]},
                    "00200010": {"vr": "SH", "Value": ["1"]}
                }
            ]
        });
        let mut env = ResponseEnvelope {
            request_details,
            response_details: ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            original_data: input.clone(),
            normalized_data: Some(input),
            normalized_snapshot: None,
        };

        let spec_path = format!(
            "{}/examples/fhir-to-dicom/transforms/dicom_to_imagingstudy_simple.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let cfg = JoltTransformMiddlewareConfig {
            spec_path,
            apply: "right".into(),
            fail_on_error: true,
        };
        let mw = JoltTransformMiddleware::new(cfg).unwrap();

        env = mw.right(env).await.unwrap();
        let out = env.normalized_data.unwrap();
        assert_eq!(
            out.get("resourceType").and_then(|v| v.as_str()),
            Some("Bundle")
        );
        assert!(out.get("entry").and_then(|v| v.as_array()).is_some());
    }
}
