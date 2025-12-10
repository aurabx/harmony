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
        let transform_config: TransformConfig = config.clone().into();
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
            // Always wrap data with context
            let transform_input = serde_json::json!({
                "data": normalized_data,
                "context": {
                    "request_details": {
                        "method": envelope.request_details.method,
                        "uri": envelope.request_details.uri,
                        "query_params": envelope.request_details.query_params,
                        "headers": envelope.request_details.headers,
                        "cookies": envelope.request_details.cookies,
                        "metadata": envelope.request_details.metadata,
                    },
                    "target_details": envelope.target_details,
                }
            });

            match self.engine.transform(transform_input) {
                Ok(transformed) => {
                    // Extract the "data" field
                    let result_data = transformed.get("data").cloned().unwrap_or(transformed.clone());

                    // Check for context updates to merge back
                    if let Some(context_out) = transformed.get("context") {
                        // Merge target_details.metadata if present in output context
                        if let Some(td_out) = context_out.get("target_details") {
                            if let Some(meta_out) = td_out.get("metadata").and_then(|m| m.as_object()) {
                                if envelope.target_details.is_none() {
                                    // This is tricky if target_details is None but we want to set metadata.
                                    // For now, we only update if target_details exists, or if we should create it?
                                    // The middleware usually runs after routing so target_details might be there.
                                    // If not, we might need to rely on request_details.metadata.
                                }
                                
                                if let Some(td) = &mut envelope.target_details {
                                    for (k, v) in meta_out {
                                        if let Some(s) = v.as_str() {
                                            td.metadata.insert(k.clone(), s.to_string());
                                        }
                                    }
                                }
                                if let Some(query_params) = td_obj.get("query_params").and_then(|v| v.as_object()) {
                                    for (k, v) in query_params {
                                        if let Some(s) = v.as_str() {
                                            target.query_params.insert(k.clone(), vec![s.to_string()]);
                                        } else if let Some(arr) = v.as_array() {
                                            let values: Vec<String> = arr.iter().filter_map(|item| item.as_str().map(|s| s.to_string())).collect();
                                            if !values.is_empty() {
                                                target.query_params.insert(k.clone(), values);
                                            }
                                        }
                                    }
                                }
                                if let Some(metadata) = td_obj.get("metadata").and_then(|v| v.as_object()) {
                                    for (k, v) in metadata {
                                        if let Some(s) = v.as_str() {
                                            target.metadata.insert(k.clone(), s.to_string());
                                        }
                                    }
                                }
                                envelope.target_details = Some(target);
                            }
                        }
                        
                        // Merge request_details.metadata if present in output context
                         if let Some(rd_out) = context_out.get("request_details") {
                             if let Some(meta_out) = rd_out.get("metadata").and_then(|m| m.as_object()) {
                                 for (k, v) in meta_out {
                                     if let Some(s) = v.as_str() {
                                         envelope.request_details.metadata.insert(k.clone(), s.to_string());
                                     }
                                 }
                             }
                         }
                    }

                    envelope.normalized_data = Some(result_data);
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
            // Always wrap data with context
            let transform_input = serde_json::json!({
                "data": normalized_data,
                "context": {
                    "request_details": {
                        "method": envelope.request_details.method,
                        "uri": envelope.request_details.uri,
                        "query_params": envelope.request_details.query_params,
                        "headers": envelope.request_details.headers,
                        "cookies": envelope.request_details.cookies,
                        "metadata": envelope.request_details.metadata,
                    },
                    "response_details": {
                        "status": envelope.response_details.status,
                        "headers": envelope.response_details.headers,
                        "metadata": envelope.response_details.metadata,
                    },
                }
            });

            tracing::debug!("JOLT transform input (response): {}", serde_json::to_string_pretty(&transform_input).unwrap_or_default());
            match self.engine.transform(transform_input) {
                Ok(transformed) => {
                    tracing::debug!("JOLT transform output (response): {}", serde_json::to_string_pretty(&transformed).unwrap_or_default());
                    // Extract the "data" field
                    let result_data = transformed.get("data").cloned().unwrap_or(transformed.clone());

                    // Check for context updates to merge back
                    if let Some(context_out) = transformed.get("context") {
                        // Merge response_details.metadata if present in output context
                         if let Some(rd_out) = context_out.get("response_details") {
                             if let Some(meta_out) = rd_out.get("metadata").and_then(|m| m.as_object()) {
                                 for (k, v) in meta_out {
                                     if let Some(s) = v.as_str() {
                                         envelope.response_details.metadata.insert(k.clone(), s.to_string());
                                     }
                                 }
                             }
                         }
                    }

                    envelope.normalized_data = Some(result_data);
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
    transforms_path: Option<&str>,
) -> Result<JoltTransformMiddlewareConfig, String> {
    let spec_path_raw = options
        .get("spec_path")
        .and_then(|v| v.as_str())
        .ok_or("Missing required 'spec_path' in transform middleware config")?
        .to_string();

    // Resolve spec_path relative to transforms_path if provided
    let spec_path = if let Some(base_path) = transforms_path {
        use std::path::Path;
        let full_path = Path::new(base_path).join(&spec_path_raw);
        full_path.to_string_lossy().to_string()
    } else {
        spec_path_raw
    };

    let apply = options
        .get("apply")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(default_apply);

    let fail_on_error = options
        .get("fail_on_error")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(default_fail_on_error);

    let _inject_context = options
        .get("inject_context")
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // Deprecated, but still parsed if present (ignored in logic as we always inject)

    Ok(JoltTransformMiddlewareConfig {
        spec_path,
        apply,
        fail_on_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{
        RequestEnvelopeBuilder, ResponseDetails, ResponseEnvelope,
    };
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
        // Since we now always inject context, the input to the transform will be:
        // { "data": { "id": 1, ... }, "context": { ... } }
        // We need to shift "data.name" -> "data.name", etc.
        let spec = json!([
            {
                "operation": "shift",
                "spec": {
                    "data": {
                        "name": "data.name",
                        "account": "data.account"
                    }
                }
            }
        ]);

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
            "name": "John Smith",
            "account": {
                "id": 1000,
                "type": "Checking"
            }
        });

        assert_eq!(result.normalized_data, Some(expected));
        assert_eq!(result.normalized_snapshot, Some(input));
    }

    #[tokio::test]
    async fn test_jolt_transform_middleware_right_only() {
        // Create a simple identity transform
        // With context injection, we want to preserve everything under "data"
        let spec = json!([
            {
                "operation": "shift",
                "spec": {
                    "data": {
                        "*": "data.&"
                    }
                }
            }
        ]);

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
        // With context injection, we need to handle the data wrapper
        let spec = json!([
            {
                "operation": "shift",
                "spec": {
                    "data": {
                        "*": "data.&"
                    }
                }
            }
        ]);
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

        let config = parse_config(&options, None).unwrap();
        assert_eq!(config.spec_path, "/path/to/spec.json");
        assert_eq!(config.apply, "both");
        assert!(!config.fail_on_error);
    }

    #[test]
    fn test_parse_config_missing_spec_path() {
        let options = HashMap::new();
        let result = parse_config(&options, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required 'spec_path'"));
    }

    #[tokio::test]
    async fn test_middleware_with_real_fhir_to_dicom_params_left() {
        use crate::models::envelope::envelope::TargetDetails;

        // Build envelope with target_details.metadata for context injection
        let mut target_metadata = HashMap::new();
        target_metadata.insert("PatientID".to_string(), "PID156695".to_string());
        target_metadata.insert("StudyInstanceUID".to_string(), "1.2.3.4.5".to_string());

        let target_details = TargetDetails {
            base_url: "http://backend.example.com".to_string(),
            method: "GET".to_string(),
            uri: "/dicom/query".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            metadata: target_metadata,
        };

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

        // Set target_details manually
        env.target_details = Some(target_details);

        // Use real spec file with context injection enabled
        let spec_path = format!(
            "{}/examples/fhir_dicom/transforms/fhir_to_dicom_params.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let cfg = JoltTransformMiddlewareConfig {
            spec_path,
            apply: "left".into(),
            fail_on_error: true,
        };
        let mw = JoltTransformMiddleware::new(cfg).unwrap();

        env = mw.left(env).await.unwrap();
        // The transform now injects context, so the output data field (extracted automatically)
        // should contain the dimse_identifier if the transform puts it there.
        // However, the updated fhir_to_dicom_params.json now outputs dimse_op to context.
        // Let's verify that dimse_op is now in target_details.metadata
        let out = env.normalized_data.unwrap();
        assert!(out.is_object(), "Output should be object");
        // dimse_identifier is still in data
        assert!(out.get("dimse_identifier").is_some());
        
        // dimse_op should be in target_details.metadata
        assert_eq!(
            env.target_details
                .as_ref()
                .and_then(|td| td.metadata.get("dimse_op").map(|s| s.as_str())),
            Some("find")
        );
    }
}
