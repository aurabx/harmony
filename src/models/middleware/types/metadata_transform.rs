use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
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
    /// What to transform: "metadata" (legacy), "target_details" (new default)
    #[serde(default = "default_transform_target")]
    pub transform_target: String,
}

fn default_apply() -> String {
    "left".to_string()
}

fn default_fail_on_error() -> bool {
    true
}

fn default_transform_target() -> String {
    "target_details".to_string()
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
    transform_target: String,
}

impl MetadataTransformMiddleware {
    pub fn new(config: MetadataTransformConfig) -> Result<Self, String> {
        let transform_config: TransformConfig = config.clone().into();
        let engine = JoltTransformEngine::new(transform_config)
            .map_err(|e| format!("Failed to create metadata transform engine: {}", e))?;

        tracing::info!(
            "Metadata transform middleware initialized (target: {})",
            config.transform_target
        );
        Ok(Self {
            engine,
            transform_target: config.transform_target,
        })
    }
}

impl MetadataTransformMiddleware {
    /// Convert JSON value to TargetDetails structure
    fn json_to_target_details(&self, json: &Value) -> Result<crate::models::envelope::envelope::TargetDetails, Error> {
        use crate::models::envelope::envelope::TargetDetails;
        
        let obj = json.as_object().ok_or_else(|| {
            Error::from("Transformed JSON must be an object for target_details")
        })?;

        // Extract fields with defaults
        let base_url = obj
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        
        let method = obj
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_string();
        
        let uri = obj
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or("/")
            .to_string();

        // Convert nested objects/arrays to HashMaps
        let headers = self.json_to_string_map(obj.get("headers"));
        let cookies = self.json_to_string_map(obj.get("cookies"));
        let query_params = self.json_to_string_vec_map(obj.get("query_params"));
        let metadata = self.json_to_string_map(obj.get("metadata"));

        Ok(TargetDetails {
            base_url,
            method,
            uri,
            headers,
            cookies,
            query_params,
            metadata,
        })
    }

    /// Convert JSON object to HashMap<String, String>
    fn json_to_string_map(&self, json: Option<&Value>) -> HashMap<String, String> {
        json.and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Convert JSON object to HashMap<String, Vec<String>>
    fn json_to_string_vec_map(&self, json: Option<&Value>) -> HashMap<String, Vec<String>> {
        json.and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| {
                        let vec = match v {
                            Value::Array(arr) => arr
                                .iter()
                                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                                .collect(),
                            Value::String(s) => vec![s.clone()],
                            _ => vec![],
                        };
                        (k.clone(), vec)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Merge transformed target_details into existing target_details
    fn merge_target_details(
        &self,
        existing: &mut crate::models::envelope::envelope::TargetDetails,
        new: &crate::models::envelope::envelope::TargetDetails,
    ) {
        // Only update non-empty/non-default values
        if !new.base_url.is_empty() {
            existing.base_url = new.base_url.clone();
        }
        if !new.method.is_empty() {
            existing.method = new.method.clone();
        }
        if !new.uri.is_empty() {
            existing.uri = new.uri.clone();
        }
        
        // Merge maps (new values override existing)
        existing.headers.extend(new.headers.clone());
        existing.cookies.extend(new.cookies.clone());
        existing.query_params.extend(new.query_params.clone());
        existing.metadata.extend(new.metadata.clone());
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

        match self.transform_target.as_str() {
            "target_details" => {
                // Build input JSON from request_details (source of query_params, headers, etc.)
                let input_json = serde_json::json!({
                    "query_params": envelope.request_details.query_params,
                    "headers": envelope.request_details.headers,
                    "cookies": envelope.request_details.cookies,
                    "metadata": envelope.request_details.metadata,
                    "method": envelope.request_details.method,
                    "uri": envelope.request_details.uri,
                });

                // Apply transform to create/modify target_details
                match self.engine.transform(input_json) {
                    Ok(transformed) => {
                        // Create or update target_details from transformed JSON
                        let target_details = self.json_to_target_details(&transformed)?;
                        
                        // Initialize target_details if not present
                        if envelope.target_details.is_none() {
                            envelope.target_details = Some(target_details);
                        } else {
                            // Merge with existing target_details
                            self.merge_target_details(envelope.target_details.as_mut().unwrap(), &target_details);
                        }
                        
                        tracing::debug!("Applied metadata transform to target_details on left side");
                    }
                    Err(e) => {
                        let error_msg = format!("Metadata transform failed on left side: {}", e);
                        if self.engine.should_fail_on_error() {
                            tracing::error!("{}", error_msg);
                            return Err(Error::from(error_msg));
                        } else {
                            tracing::warn!("{}, continuing without target_details modification", error_msg);
                        }
                    }
                }
            }
            "metadata" => {
                // Legacy behavior: transform request_details.metadata only
                let metadata_json: serde_json::Value = {
                    let mut map = serde_json::Map::new();
                    for (key, value) in envelope.request_details.metadata.iter() {
                        map.insert(key.clone(), serde_json::Value::String(value.clone()));
                    }
                    serde_json::Value::Object(map)
                };

                match self.engine.transform(metadata_json) {
                    Ok(transformed) => {
                        if let Some(obj) = transformed.as_object() {
                            for (key, value) in obj.iter() {
                                if let Some(string_value) = value.as_str() {
                                    envelope
                                        .request_details
                                        .metadata
                                        .insert(key.clone(), string_value.to_string());
                                }
                            }
                        }
                        tracing::debug!("Applied metadata transform on left side (legacy mode)");
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
            }
            _ => {
                return Err(Error::from(format!(
                    "Unknown transform_target: {}. Must be 'metadata' or 'target_details'",
                    self.transform_target
                )));
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

        // Convert metadata HashMap<String, String> to serde_json::Value
        let metadata_json: serde_json::Value = {
            let mut map = serde_json::Map::new();
            for (key, value) in envelope.response_details.metadata.iter() {
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
                            envelope
                                .response_details
                                .metadata
                                .insert(key.clone(), string_value.to_string());
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
    transforms_path: Option<&str>,
) -> Result<MetadataTransformConfig, String> {
    let spec_path_raw = options
        .get("spec_path")
        .and_then(|v| v.as_str())
        .ok_or("Missing required 'spec_path' in metadata_transform middleware config")?
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

    let transform_target = options
        .get("transform_target")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(default_transform_target);

    Ok(MetadataTransformConfig {
        spec_path,
        apply,
        fail_on_error,
        transform_target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::RequestEnvelopeBuilder;
    use serde_json::json;
    use std::fs;
    use tempfile::NamedTempFile;

    fn create_test_envelope(metadata: HashMap<String, String>) -> RequestEnvelope<Value> {
        RequestEnvelopeBuilder::new()
            .method("POST")
            .uri("/test")
            .metadata(metadata)
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap()
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
            transform_target: "metadata".to_string(),
        };

        let middleware = MetadataTransformMiddleware::new(config).unwrap();

        let envelope = create_test_envelope(HashMap::new());
        let result = middleware.left(envelope).await.unwrap();

        assert_eq!(
            result.request_details.metadata.get("dimse_op"),
            Some(&"find".to_string())
        );
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
            transform_target: "metadata".to_string(),
        };

        let middleware = MetadataTransformMiddleware::new(config).unwrap();

        let envelope = create_test_envelope(HashMap::new());

        // Left should do nothing
        let left_result = middleware.left(envelope).await.unwrap();
        assert!(!left_result
            .request_details
            .metadata
            .contains_key("dimse_op"));

        // Convert to ResponseEnvelope for right() method
        let response_envelope = crate::models::envelope::envelope::ResponseEnvelope {
            request_details: left_result.request_details,
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            original_data: left_result.original_data,
            normalized_data: left_result.normalized_data,
            normalized_snapshot: left_result.normalized_snapshot,
        };

        // Right should apply transform
        let right_result = middleware.right(response_envelope).await.unwrap();
        // After refactor, right() transforms response_details.metadata, not request_details.metadata
        assert_eq!(
            right_result.response_details.metadata.get("dimse_op"),
            Some(&"find".to_string())
        );
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
            transform_target: "metadata".to_string(),
        };

        let middleware = MetadataTransformMiddleware::new(config).unwrap();

        let mut metadata = HashMap::new();
        metadata.insert("existing_key".to_string(), "existing_value".to_string());

        let envelope = create_test_envelope(metadata);
        let result = middleware.left(envelope).await.unwrap();

        // Should preserve existing metadata and add new
        assert_eq!(
            result.request_details.metadata.get("existing_key"),
            Some(&"existing_value".to_string())
        );
        assert_eq!(
            result.request_details.metadata.get("dimse_op"),
            Some(&"find".to_string())
        );
    }

    #[tokio::test]
    async fn test_non_string_outputs_are_ignored() {
        // Set a string and a numeric value; only string should be written back
        let spec = json!([{
            "operation": "default",
            "spec": {
                "dimse_op": "get",
                "num": 123
            }
        }]);
        let temp = NamedTempFile::new().unwrap();
        fs::write(&temp, serde_json::to_string_pretty(&spec).unwrap()).unwrap();
        let cfg = MetadataTransformConfig {
            spec_path: temp.path().to_string_lossy().to_string(),
            apply: "left".to_string(),
            fail_on_error: true,
            transform_target: "metadata".to_string(),
        };
        let mw = MetadataTransformMiddleware::new(cfg).unwrap();

        let env = create_test_envelope(HashMap::new());
        let out = mw.left(env).await.unwrap();
        assert_eq!(
            out.request_details.metadata.get("dimse_op"),
            Some(&"get".to_string())
        );
        assert!(out.request_details.metadata.get("num").is_none());
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
    fn test_parse_config_defaults() {
        let mut options = HashMap::new();
        options.insert("spec_path".to_string(), json!("/path/to/spec.json"));

        let config = parse_config(&options, None).unwrap();
        assert_eq!(config.spec_path, "/path/to/spec.json");
        assert_eq!(config.apply, "left"); // default
        assert!(config.fail_on_error); // default
    }

    #[test]
    fn test_parse_config_missing_spec_path() {
        let options = HashMap::new();
        let result = parse_config(&options, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required 'spec_path'"));
    }

    #[tokio::test]
    async fn test_metadata_middleware_with_real_spec_left_sets_dimse() {
        let env = RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/fhir/ImagingStudy")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let spec_path = format!(
            "{}/samples/jolt/metadata_set_dimse_op.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let cfg = MetadataTransformConfig {
            spec_path,
            apply: "left".into(),
            fail_on_error: true,
            transform_target: "metadata".to_string(),
        };
        let mw = MetadataTransformMiddleware::new(cfg).unwrap();
        let out = mw.left(env).await.unwrap();
        assert_eq!(
            out.request_details.metadata.get("dimse_op"),
            Some(&"find".to_string())
        );
    }
}
