use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
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
}

fn default_apply() -> String { "left".to_string() }
fn default_timeout() -> u64 { 5 }

pub struct WebhookMiddleware {
    cfg: WebhookConfig,
}

impl WebhookMiddleware {
    pub fn new(cfg: WebhookConfig) -> Self { Self { cfg } }

    fn should_left(&self) -> bool {
        self.cfg.apply.eq_ignore_ascii_case("left") || self.cfg.apply.eq_ignore_ascii_case("both")
    }
    fn should_right(&self) -> bool {
        self.cfg.apply.eq_ignore_ascii_case("right") || self.cfg.apply.eq_ignore_ascii_case("both")
    }

    fn redact_headers(&self, headers: &HashMap<String, String>) -> HashMap<String, String> {
        if self.cfg.redact_headers.is_empty() { return headers.clone(); }
        let set: Vec<String> = self.cfg.redact_headers.iter().map(|s| s.to_lowercase()).collect();
        let mut redacted = headers.clone();
        for (k, v) in redacted.iter_mut() {
            if set.iter().any(|rk| rk == &k.to_lowercase()) {
                *v = "<redacted>".to_string();
            }
        }
        redacted
    }

    fn redact_metadata(&self, metadata: &HashMap<String, String>) -> HashMap<String, String> {
        if self.cfg.redact_metadata.is_empty() { return metadata.clone(); }
        let set: Vec<String> = self.cfg.redact_metadata.iter().map(|s| s.to_lowercase()).collect();
        let mut redacted = metadata.clone();
        for (k, v) in redacted.iter_mut() {
            if set.iter().any(|rk| rk == &k.to_lowercase()) {
                *v = "<redacted>".to_string();
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

/// Parse configuration from HashMap for middleware registry
pub fn parse_config(options: &HashMap<String, Value>) -> Result<WebhookConfig, String> {
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

    Ok(WebhookConfig { endpoint, apply, redact_headers, redact_metadata, timeout_secs, instance_name, auth_def })
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
}