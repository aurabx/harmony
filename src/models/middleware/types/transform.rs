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
    /// Whether to debug log transform input and output
    #[serde(default = "default_debug")]
    pub debug: bool,
}

fn default_apply() -> String {
    "both".to_string()
}

fn default_fail_on_error() -> bool {
    true
}

fn default_debug() -> bool {
    false
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
    debug: bool,
}

impl JoltTransformMiddleware {
    pub fn new(config: JoltTransformMiddlewareConfig) -> Result<Self, String> {
        let transform_config: TransformConfig = config.clone().into();
        let engine = JoltTransformEngine::new(transform_config)
            .map_err(|e| format!("Failed to create JOLT transform engine: {}", e))?;

        tracing::info!("JOLT transform middleware initialized");
        Ok(Self {
            engine,
            debug: config.debug,
        })
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

            if self.debug && tracing::enabled!(tracing::Level::DEBUG) {
                tracing::debug!("JOLT transform input (request): {}", serde_json::to_string_pretty(&transform_input).unwrap_or_default());
            }

            match self.engine.transform(transform_input) {
                Ok(transformed) => {
                    if self.debug && tracing::enabled!(tracing::Level::DEBUG) {
                        tracing::debug!("JOLT transform output (request): {}", serde_json::to_string_pretty(&transformed).unwrap_or_default());
                    }
                    // Extract the "data" field
                    let result_data = transformed.get("data").cloned().unwrap_or(transformed.clone());

                    // Check for context updates to merge back
                    if let Some(context_out) = transformed.get("context") {
                        // Merge target_details if present in output context
                        if let Some(td_out) = context_out.get("target_details") {
                            if let Ok(td_from_transform) = serde_json::from_value::<crate::models::envelope::envelope::TargetDetails>(td_out.clone()) {
                                // Transform provided full target_details - use it
                                envelope.target_details = Some(td_from_transform);
                            } else if td_out.is_object() {
                                // Partial target_details - merge into existing or create new
                                let td_obj = td_out.as_object().unwrap();
                                let mut target = envelope.target_details.take().unwrap_or_else(|| {
                                    crate::models::envelope::envelope::TargetDetails {
                                        method: "GET".to_string(),
                                        uri: String::new(),
                                        base_url: String::new(),
                                        headers: HashMap::new(),
                                        cookies: HashMap::new(),
                                        query_params: HashMap::new(),
                                        metadata: HashMap::new(),
                                    }
                                });
                                
                                // Merge individual fields
                                if let Some(method) = td_obj.get("method").and_then(|v| v.as_str()) {
                                    target.method = method.to_string();
                                }
                                if let Some(uri) = td_obj.get("uri").and_then(|v| v.as_str()) {
                                    target.uri = uri.to_string();
                                    // If the new URI contains query params, clear existing query_params
                                    // to avoid duplication when full_url() appends them
                                    if uri.contains('?') {
                                        target.query_params.clear();
                                    }
                                }
                                if let Some(base_url) = td_obj.get("base_url").and_then(|v| v.as_str()) {
                                    target.base_url = base_url.to_string();
                                }
                                if let Some(headers) = td_obj.get("headers").and_then(|v| v.as_object()) {
                                    for (k, v) in headers {
                                        if let Some(s) = v.as_str() {
                                            target.headers.insert(k.clone(), s.to_string());
                                        }
                                    }
                                }
                                if let Some(query_params) = td_obj.get("query_params").and_then(|v| v.as_object()) {
                                    // Replace existing query_params entirely (not merge)
                                    target.query_params.clear();
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

            if self.debug && tracing::enabled!(tracing::Level::DEBUG) {
                tracing::debug!("JOLT transform input (response): {}", serde_json::to_string_pretty(&transform_input).unwrap_or_default());
            }
            match self.engine.transform(transform_input) {
                Ok(transformed) => {
                    if self.debug && tracing::enabled!(tracing::Level::DEBUG) {
                        tracing::debug!("JOLT transform output (response): {}", serde_json::to_string_pretty(&transformed).unwrap_or_default());
                    }
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

    let debug = options
        .get("debug")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(default_debug);

    Ok(JoltTransformMiddlewareConfig {
        spec_path,
        apply,
        fail_on_error,
        debug,
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
            debug: false,
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
            debug: false,
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
            debug: false,
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

    #[test]
    fn test_parse_config_with_debug() {
        let mut options = HashMap::new();
        options.insert("spec_path".to_string(), json!("/path/to/spec.json"));
        options.insert("debug".to_string(), json!(true));

        let config = parse_config(&options, None).unwrap();
        assert_eq!(config.spec_path, "/path/to/spec.json");
        assert!(config.debug);
    }

    #[test]
    fn test_parse_config_debug_defaults_to_false() {
        let mut options = HashMap::new();
        options.insert("spec_path".to_string(), json!("/path/to/spec.json"));

        let config = parse_config(&options, None).unwrap();
        assert!(!config.debug);
    }

    #[tokio::test]
    async fn test_transform_with_debug_enabled() {
        // Test that debug flag is properly set and doesn't break execution
        let spec = json!([
            {
                "operation": "shift",
                "spec": {
                    "data": {
                        "name": "data.name"
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
            debug: true,
        };

        let middleware = JoltTransformMiddleware::new(config).unwrap();
        assert!(middleware.debug);

        let input = json!({"name": "Alice"});
        let envelope = create_test_envelope(input.clone());
        let result = middleware.left(envelope).await.unwrap();

        // Verify the transform still works with debug enabled
        assert_eq!(
            result.normalized_data,
            Some(json!({"name": "Alice"}))
        );
    }

}
