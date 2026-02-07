use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::redaction::SensitiveFieldMatcher;
use crate::utils::Error;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct WebhookConfig {
    pub endpoint: String,
    #[serde(default = "default_apply")] pub apply: String,
    #[serde(default)] pub redact_headers: Vec<String>,
    #[serde(default)] pub redact_metadata: Vec<String>,
    #[serde(default = "default_timeout")] pub timeout_secs: u64,
    #[serde(default)] pub instance_name: String,
    #[serde(skip)] pub auth_def: Option<crate::models::connection::AuthenticationDefinition>,
    /// Global sensitive field patterns from proxy config (regex patterns)
    #[serde(default)] pub sensitive_field_patterns: Vec<String>,
}

fn default_apply() -> String { "left".to_string() }
fn default_timeout() -> u64 { 5 }

pub struct WebhookMiddleware {
    cfg: WebhookConfig,
    /// Compiled pattern matcher for sensitive field detection
    sensitive_matcher: SensitiveFieldMatcher,
}

impl WebhookMiddleware {
    pub fn new(cfg: WebhookConfig) -> Self {
        let sensitive_matcher = SensitiveFieldMatcher::new(&cfg.sensitive_field_patterns);
        Self { cfg, sensitive_matcher }
    }

    fn should_left(&self) -> bool {
        self.cfg.apply.eq_ignore_ascii_case("left") || self.cfg.apply.eq_ignore_ascii_case("both")
    }
    fn should_right(&self) -> bool {
        self.cfg.apply.eq_ignore_ascii_case("right") || self.cfg.apply.eq_ignore_ascii_case("both")
    }

    fn redact_headers(&self, headers: &HashMap<String, String>) -> HashMap<String, String> {
        // First apply pattern-based redaction from global sensitive_field_patterns
        let mut redacted = self.sensitive_matcher.redact_headers(headers);
        
        // Then apply explicit redact_headers list (case-insensitive exact match)
        if !self.cfg.redact_headers.is_empty() {
            let set: Vec<String> = self.cfg.redact_headers.iter().map(|s| s.to_lowercase()).collect();
            for (k, v) in redacted.iter_mut() {
                if set.iter().any(|rk| rk == &k.to_lowercase()) {
                    *v = "<redacted>".to_string();
                }
            }
        }
        redacted
    }

    fn redact_metadata(&self, metadata: &HashMap<String, String>) -> HashMap<String, String> {
        // First apply pattern-based redaction from global sensitive_field_patterns
        let mut redacted = self.sensitive_matcher.redact_metadata(metadata);
        
        // Then apply explicit redact_metadata list (case-insensitive exact match)
        if !self.cfg.redact_metadata.is_empty() {
            let set: Vec<String> = self.cfg.redact_metadata.iter().map(|s| s.to_lowercase()).collect();
            for (k, v) in redacted.iter_mut() {
                if set.iter().any(|rk| rk == &k.to_lowercase()) {
                    *v = "<redacted>".to_string();
                }
            }
        }
        redacted
    }

    fn build_extra_from_metadata(&self, metadata: &HashMap<String, String>) -> Value {
        let key = format!("webhook.{}", self.cfg.instance_name);
        if let Some(raw) = metadata.get(&key) {
            // Try parse as JSON string first
            if let Ok(json_val) = serde_json::from_str::<Value>(raw) {
                return json_val;
            }
            return Value::String(raw.clone());
        }
        Value::Null
    }

    fn apply_auth(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        if let Some(auth) = &self.cfg.auth_def {
            let mut opts = HashMap::new();
            if let Ok(val) = serde_json::to_value(auth) {
                opts.insert("authentication_def".to_string(), val);
            }
            return crate::models::services::backend_auth::apply_backend_authentication(builder, &opts, "WEBHOOK");
        }
        builder
    }

    fn client(&self) -> Result<reqwest::Client, Error> {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.cfg.timeout_secs))
            .build()
            .map_err(|e| Error::from(format!("Failed to create HTTP client: {}", e)))
    }
}

#[async_trait]
impl Middleware for WebhookMiddleware {
    async fn left(
        &self,
        envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<Value>, Error> {
        if !self.should_left() { return Ok(envelope); }

        let client = self.client()?;
        let headers = self.redact_headers(&envelope.request_details.headers);
        let metadata = self.redact_metadata(&envelope.request_details.metadata);
        let payload = serde_json::json!({
            "middleware": "webhook",
            "name": self.cfg.instance_name,
            "side": "left",
            "request": {
                "method": envelope.request_details.method,
                "uri": envelope.request_details.uri,
                "headers": headers,
                "cookies": envelope.request_details.cookies, // not redacting unless configured at header level
                "query_params": envelope.request_details.query_params,
                "metadata": metadata,
                "content_metadata": envelope.request_details.content_metadata
            },
            "extra": self.build_extra_from_metadata(&envelope.request_details.metadata)
        });

        let endpoint = self.cfg.endpoint.clone();
        let req = client.post(endpoint).header("content-type", "application/json");
        let req = self.apply_auth(req).json(&payload);

        tokio::spawn(async move {
            if let Err(e) = req.send().await {
                tracing::warn!(target: "harmony.webhook", "webhook post failed: {}", e);
            }
        });

        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<Value>,
    ) -> Result<ResponseEnvelope<Value>, Error> {
        if !self.should_right() { return Ok(envelope); }

        let client = self.client()?;
        let headers = self.redact_headers(&envelope.request_details.headers);
        let metadata = self.redact_metadata(&envelope.request_details.metadata);
        let payload = serde_json::json!({
            "middleware": "webhook",
            "name": self.cfg.instance_name,
            "side": "right",
            "request": {
                "method": envelope.request_details.method,
                "uri": envelope.request_details.uri,
                "headers": headers,
                "cookies": envelope.request_details.cookies,
                "query_params": envelope.request_details.query_params,
                "metadata": metadata,
                "content_metadata": envelope.request_details.content_metadata
            },
            "extra": self.build_extra_from_metadata(&envelope.request_details.metadata)
        });

        let endpoint = self.cfg.endpoint.clone();
        let req = client.post(endpoint).header("content-type", "application/json");
        let req = self.apply_auth(req).json(&payload);

        tokio::spawn(async move {
            if let Err(e) = req.send().await {
                tracing::warn!(target: "harmony.webhook", "webhook post failed: {}", e);
            }
        });

        Ok(envelope)
    }
}

/// Parse configuration from HashMap for middleware registry.
pub fn parse_config(options: &HashMap<String, Value>) -> Result<WebhookConfig, String> {
    parse_config_with_patterns(options, &[])
}

/// Parse configuration with optional global sensitive field patterns.
pub fn parse_config_with_patterns(
    options: &HashMap<String, Value>,
    global_sensitive_patterns: &[String],
) -> Result<WebhookConfig, String> {
    let endpoint = options
        .get("endpoint")
        .and_then(|v| v.as_str())
        .ok_or("Missing required 'endpoint' in webhook middleware config")?
        .to_string();

    let apply = options
        .get("apply").and_then(|v| v.as_str()).map(|s| s.to_string())
        .unwrap_or_else(default_apply);

    let redact_headers = options
        .get("redact_headers")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let redact_metadata = options
        .get("redact_metadata")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let timeout_secs = options
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(default_timeout);

    let instance_name = options
        .get("__instance_name")
        .and_then(|v| v.as_str())
        .unwrap_or("webhook")
        .to_string();

    let auth_def = options
        .get("authentication_def")
        .and_then(|v| serde_json::from_value::<crate::models::connection::AuthenticationDefinition>(v.clone()).ok());

    // Use global patterns passed from proxy config
    let sensitive_field_patterns = global_sensitive_patterns.to_vec();

    Ok(WebhookConfig { 
        endpoint, 
        apply, 
        redact_headers, 
        redact_metadata, 
        timeout_secs, 
        instance_name, 
        auth_def,
        sensitive_field_patterns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_defaults() {
        let mut opts = HashMap::new();
        opts.insert("endpoint".into(), json!("https://example.com/hook"));
        let cfg = parse_config(&opts).unwrap();
        assert_eq!(cfg.endpoint, "https://example.com/hook");
        assert_eq!(cfg.apply.to_lowercase(), "left");
        assert_eq!(cfg.timeout_secs, 5);
    }

    #[test]
    fn test_requires_endpoint() {
        let opts = HashMap::new();
        assert!(parse_config(&opts).is_err());
    }

    #[test]
    fn test_parse_config_with_global_patterns() {
        let mut opts = HashMap::new();
        opts.insert("endpoint".into(), json!("https://example.com/hook"));
        
        let global_patterns = vec![".*ssn.*".to_string(), ".*password.*".to_string()];
        let cfg = parse_config_with_patterns(&opts, &global_patterns).unwrap();
        
        assert_eq!(cfg.sensitive_field_patterns, global_patterns);
    }

    #[test]
    fn test_sensitive_patterns_redact_headers() {
        let cfg = WebhookConfig {
            endpoint: "https://example.com".to_string(),
            apply: "left".to_string(),
            redact_headers: vec![],
            redact_metadata: vec![],
            timeout_secs: 5,
            instance_name: "test".to_string(),
            auth_def: None,
            sensitive_field_patterns: vec![".*secret.*".to_string(), ".*password.*".to_string()],
        };
        let mw = WebhookMiddleware::new(cfg);

        let mut headers = HashMap::new();
        headers.insert("X-Api-Secret".to_string(), "secret-value".to_string());
        headers.insert("X-Password".to_string(), "pass123".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let redacted = mw.redact_headers(&headers);

        assert_eq!(redacted.get("X-Api-Secret").unwrap(), "<redacted>");
        assert_eq!(redacted.get("X-Password").unwrap(), "<redacted>");
        assert_eq!(redacted.get("Content-Type").unwrap(), "application/json");
    }

    #[test]
    fn test_sensitive_patterns_redact_metadata() {
        let cfg = WebhookConfig {
            endpoint: "https://example.com".to_string(),
            apply: "left".to_string(),
            redact_headers: vec![],
            redact_metadata: vec![],
            timeout_secs: 5,
            instance_name: "test".to_string(),
            auth_def: None,
            sensitive_field_patterns: vec![".*ssn.*".to_string()],
        };
        let mw = WebhookMiddleware::new(cfg);

        let mut metadata = HashMap::new();
        metadata.insert("patient_ssn".to_string(), "123-45-6789".to_string());
        metadata.insert("patient_id".to_string(), "12345".to_string());

        let redacted = mw.redact_metadata(&metadata);

        assert_eq!(redacted.get("patient_ssn").unwrap(), "<redacted>");
        assert_eq!(redacted.get("patient_id").unwrap(), "12345");
    }

    #[test]
    fn test_combined_explicit_and_pattern_redaction() {
        let cfg = WebhookConfig {
            endpoint: "https://example.com".to_string(),
            apply: "left".to_string(),
            redact_headers: vec!["authorization".to_string()],
            redact_metadata: vec!["token".to_string()],
            timeout_secs: 5,
            instance_name: "test".to_string(),
            auth_def: None,
            sensitive_field_patterns: vec![".*secret.*".to_string()],
        };
        let mw = WebhookMiddleware::new(cfg);

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token".to_string());
        headers.insert("X-Api-Secret".to_string(), "secret123".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let redacted = mw.redact_headers(&headers);

        assert_eq!(redacted.get("Authorization").unwrap(), "<redacted>");
        assert_eq!(redacted.get("X-Api-Secret").unwrap(), "<redacted>");
        assert_eq!(redacted.get("Content-Type").unwrap(), "application/json");
    }
}