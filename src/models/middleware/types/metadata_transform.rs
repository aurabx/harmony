use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use harmony_transform::{JoltTransformEngine, TransformConfig};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct MetadataTransformConfig {
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
    "left".to_string()
}

fn default_fail_on_error() -> bool {
    true
}

impl From<MetadataTransformConfig> for TransformConfig {
    fn from(config: MetadataTransformConfig) -> Self {
        TransformConfig {
            spec_path: config.spec_path,
            apply: config.apply,
            fail_on_error: config.fail_on_error,
        }
    }
}

pub struct MetadataTransformMiddleware {
    engine: JoltTransformEngine,
}

impl MetadataTransformMiddleware {
    pub fn new(config: MetadataTransformConfig) -> Result<Self, String> {
        let transform_config: TransformConfig = config.into();
        let engine = JoltTransformEngine::new(transform_config)
            .map_err(|e| format!("Failed to create metadata transform engine: {}", e))?;

        tracing::info!("Metadata transform middleware initialized");
        Ok(Self { engine })
    }
}

#[async_trait]
impl Middleware for MetadataTransformMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        if !self.engine.should_apply_left() {
            return Ok(envelope);
        }

        // Convert metadata HashMap<String, String> to serde_json::Value
        let metadata_json: serde_json::Value = {
            let mut map = serde_json::Map::new();
            for (key, value) in envelope.request_details.metadata.iter() {
                map.insert(key.clone(), serde_json::Value::String(value.clone()));
            }
            serde_json::Value::Object(map)
        };

        // Apply transform to metadata
        match self.engine.transform(metadata_json) {
            Ok(transformed) => {
                // Update metadata with transformed values (only string values)
                if let Some(obj) = transformed.as_object() {
                    for (key, value) in obj.iter() {
                        if let Some(string_value) = value.as_str() {
                            envelope.request_details.metadata.insert(key.clone(), string_value.to_string());
                        }
                        // Ignore non-string values
                    }
                }
                tracing::debug!("Applied metadata transform on left side");
            }
            Err(e) => {
                let error_msg = format!("Metadata transform failed on left side: {}", e);
                if self.engine.should_fail_on_error() {
                    tracing::error!("{}", error_msg);
                    return Err(Error::from(error_msg));
                } else {
                    tracing::warn!("{}, continuing with original metadata", error_msg);
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

        // Convert metadata HashMap<String, String> to serde_json::Value
        let metadata_json: serde_json::Value = {
            let mut map = serde_json::Map::new();
            for (key, value) in envelope.request_details.metadata.iter() {
                map.insert(key.clone(), serde_json::Value::String(value.clone()));
            }
            serde_json::Value::Object(map)
        };

        // Apply transform to metadata
        match self.engine.transform(metadata_json) {
            Ok(transformed) => {
                // Update metadata with transformed values (only string values)
                if let Some(obj) = transformed.as_object() {
                    for (key, value) in obj.iter() {
                        if let Some(string_value) = value.as_str() {
                            envelope.request_details.metadata.insert(key.clone(), string_value.to_string());
                        }
                        // Ignore non-string values
                    }
                }
                tracing::debug!("Applied metadata transform on right side");
            }
            Err(e) => {
                let error_msg = format!("Metadata transform failed on right side: {}", e);
                if self.engine.should_fail_on_error() {
                    tracing::error!("{}", error_msg);
                    return Err(Error::from(error_msg));
                } else {
                    tracing::warn!("{}, continuing with original metadata", error_msg);
                }
            }
        }

        Ok(envelope)
    }
}

/// Parse configuration from HashMap for middleware registry
pub fn parse_config(
    options: &HashMap<String, Value>,
) -> Result<MetadataTransformConfig, String> {
    let spec_path = options
        .get("spec_path")
        .and_then(|v| v.as_str())
        .ok_or("Missing required 'spec_path' in metadata_transform middleware config")?
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

    Ok(MetadataTransformConfig {
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

    fn create_test_envelope(metadata: HashMap<String, String>) -> RequestEnvelope<Value> {
        let request_details = RequestDetails {
            method: "POST".to_string(),
            uri: "/test".to_string(),
            headers: Default::default(),
            cookies: Default::default(),
            query_params: Default::default(),
            cache_status: None,
            metadata,
        };

        RequestEnvelope {
            request_details,
            original_data: serde_json::Value::Null,
            normalized_data: Some(serde_json::Value::Null),
            normalized_snapshot: None,
        }
    }

    #[tokio::test]
    async fn test_sets_dimse_op_on_left() {
        // Create a temporary JOLT spec file
        let spec = json!([{
            "operation": "default",
            "spec": {
                "dimse_op": "find"
            }
        }]);

        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, serde_json::to_string_pretty(&spec).unwrap()).unwrap();

        let config = MetadataTransformConfig {
            spec_path: temp_file.path().to_string_lossy().to_string(),
            apply: "left".to_string(),
            fail_on_error: true,
        };

        let middleware = MetadataTransformMiddleware::new(config).unwrap();

        let envelope = create_test_envelope(HashMap::new());
        let result = middleware.left(envelope).await.unwrap();

        assert_eq!(result.request_details.metadata.get("dimse_op"), Some(&"find".to_string()));
    }

    #[tokio::test]
    async fn test_apply_right_only() {
        // Create a temporary JOLT spec file
        let spec = json!([{
            "operation": "default",
            "spec": {
                "dimse_op": "find"
            }
        }]);

        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, serde_json::to_string_pretty(&spec).unwrap()).unwrap();

        let config = MetadataTransformConfig {
            spec_path: temp_file.path().to_string_lossy().to_string(),
            apply: "right".to_string(),
            fail_on_error: true,
        };

        let middleware = MetadataTransformMiddleware::new(config).unwrap();

        let envelope = create_test_envelope(HashMap::new());
        
        // Left should do nothing
        let left_result = middleware.left(envelope).await.unwrap();
        assert!(!left_result.request_details.metadata.contains_key("dimse_op"));

        // Right should apply transform
        let right_result = middleware.right(left_result).await.unwrap();
        assert_eq!(right_result.request_details.metadata.get("dimse_op"), Some(&"find".to_string()));
    }

    #[tokio::test]
    async fn test_preserves_existing_metadata() {
        // Create a temporary JOLT spec file
        let spec = json!([{
            "operation": "default",
            "spec": {
                "dimse_op": "find"
            }
        }]);

        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, serde_json::to_string_pretty(&spec).unwrap()).unwrap();

        let config = MetadataTransformConfig {
            spec_path: temp_file.path().to_string_lossy().to_string(),
            apply: "left".to_string(),
            fail_on_error: true,
        };

        let middleware = MetadataTransformMiddleware::new(config).unwrap();

        let mut metadata = HashMap::new();
        metadata.insert("existing_key".to_string(), "existing_value".to_string());
        
        let envelope = create_test_envelope(metadata);
        let result = middleware.left(envelope).await.unwrap();

        // Should preserve existing metadata and add new
        assert_eq!(result.request_details.metadata.get("existing_key"), Some(&"existing_value".to_string()));
        assert_eq!(result.request_details.metadata.get("dimse_op"), Some(&"find".to_string()));
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
    fn test_parse_config_defaults() {
        let mut options = HashMap::new();
        options.insert("spec_path".to_string(), json!("/path/to/spec.json"));

        let config = parse_config(&options).unwrap();
        assert_eq!(config.spec_path, "/path/to/spec.json");
        assert_eq!(config.apply, "left"); // default
        assert!(config.fail_on_error); // default
    }

    #[test]
    fn test_parse_config_missing_spec_path() {
        let options = HashMap::new();
        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required 'spec_path'"));
    }
}