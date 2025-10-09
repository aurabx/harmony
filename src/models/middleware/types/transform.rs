use crate::models::envelope::envelope::RequestEnvelope;
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
                    envelope.original_data = envelope.normalized_data.clone().unwrap_or(serde_json::Value::Null);
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
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        if !self.engine.should_apply_right() {
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
                    envelope.original_data = envelope.normalized_data.clone().unwrap_or(serde_json::Value::Null);
                    tracing::debug!("Applied JOLT transform on right side");
                }
                Err(e) => {
                    let error_msg = format!("JOLT transform failed on right side: {}", e);
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
pub fn parse_config(options: &HashMap<String, Value>) -> Result<JoltTransformMiddlewareConfig, String> {
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
    use crate::models::envelope::envelope::RequestDetails;
    use serde_json::json;
    use std::fs;
    use tempfile::NamedTempFile;

    fn create_test_envelope(data: Value) -> RequestEnvelope<Value> {
        let request_details = RequestDetails {
            method: "POST".to_string(),
            uri: "/test".to_string(),
            headers: Default::default(),
            cookies: Default::default(),
            query_params: Default::default(),
            cache_status: None,
            metadata: Default::default(),
        };

        RequestEnvelope {
            request_details,
            original_data: data.clone(),
            normalized_data: Some(data),
            normalized_snapshot: None,
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

        // Should apply transform on right
        let right_result = middleware.right(left_result).await.unwrap();
        assert_eq!(right_result.normalized_data, Some(input.clone())); // Identity transform
        assert_eq!(right_result.normalized_snapshot, Some(input)); // Snapshot created on right
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
}