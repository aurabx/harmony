use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct LogDumpMiddlewareConfig {
    /// when to dump: "left", "right", or "both" (default)
    #[serde(default = "default_apply")] 
    pub apply: String,
    /// pretty print JSON
    #[serde(default = "default_pretty")] 
    pub pretty: bool,
    /// maximum number of bytes to include from large fields like original_data when serialized
    #[serde(default = "default_max_bytes")] 
    pub max_bytes: usize,
    /// redact values for these header names (case-insensitive)
    #[serde(default)]
    pub redact_headers: Vec<String>,
    /// redact values for these metadata keys
    #[serde(default)]
    pub redact_metadata: Vec<String>,
    /// redact values for these normalized_data fields (dot path notation)
    #[serde(default)]
    pub redact_data_fields: Vec<String>,
    /// optional label to include in log lines to distinguish multiple dump points
    #[serde(default)]
    pub label: String,
}

fn default_apply() -> String { "both".to_string() }
fn default_pretty() -> bool { true }
fn default_max_bytes() -> usize { 64 * 1024 }

pub struct LogDumpMiddleware {
    cfg: LogDumpMiddlewareConfig,
}

impl LogDumpMiddleware {
    pub fn new(cfg: LogDumpMiddlewareConfig) -> Self { Self { cfg } }

    fn should_left(&self) -> bool {
        self.cfg.apply.eq_ignore_ascii_case("left") || self.cfg.apply.eq_ignore_ascii_case("both")
    }
    fn should_right(&self) -> bool {
        self.cfg.apply.eq_ignore_ascii_case("right") || self.cfg.apply.eq_ignore_ascii_case("both")
    }

    fn redact_headers(&self, headers: HashMap<String, String>) -> HashMap<String, String> {
        if self.cfg.redact_headers.is_empty() { return headers; }
        let set: Vec<String> = self.cfg.redact_headers.iter().map(|s| s.to_lowercase()).collect();
        
        let mut redacted = headers;
        for (k, v) in redacted.iter_mut() {
            if set.iter().any(|rk| rk == &k.to_lowercase()) {
                *v = "<redacted>".to_string();
            }
        }
        redacted
    }

    fn redact_metadata(&self, metadata: HashMap<String, String>) -> HashMap<String, String> {
        if self.cfg.redact_metadata.is_empty() { return metadata; }
        let set: Vec<String> = self.cfg.redact_metadata.iter().map(|s| s.to_lowercase()).collect();
        
        let mut redacted = metadata;
        for (k, v) in redacted.iter_mut() {
            if set.iter().any(|rk| rk == &k.to_lowercase()) {
                *v = "<redacted>".to_string();
            }
        }
        redacted
    }

    fn redact_normalized_data(&self, mut data: Value) -> Value {
        if self.cfg.redact_data_fields.is_empty() { return data; }
        
        for field_path in &self.cfg.redact_data_fields {
            let parts: Vec<&str> = field_path.split('.').collect();
            self.redact_field_by_path(&mut data, &parts);
        }
        data
    }
    
    fn redact_field_by_path(&self, data: &mut Value, path: &[&str]) {
        if path.is_empty() { return; }
        
        match data {
            Value::Object(ref mut map) => {
                if let Some(next_val) = map.get_mut(path[0]) {
                    if path.len() == 1 {
                        // Final component - redact this value
                        *next_val = Value::String("<redacted>".to_string());
                    } else {
                        // More components to traverse
                        self.redact_field_by_path(next_val, &path[1..]);
                    }
                }
            },
            Value::Array(ref mut arr) => {
                for item in arr.iter_mut() {
                    self.redact_field_by_path(item, path);
                }
            },
            _ => {} // Can't traverse into primitives
        }
    }

    fn truncate_bytes(&self, s: String) -> String {
        let bytes = s.as_bytes();
        if bytes.len() <= self.cfg.max_bytes { return s; }
        let mut truncated = bytes[..self.cfg.max_bytes].to_vec();
        // ensure valid UTF-8 by backing off if needed
        while std::str::from_utf8(&truncated).is_err() {
            truncated.pop();
            if truncated.is_empty() { break; }
        }
        let mut out = String::from_utf8(truncated).unwrap_or_default();
        out.push_str(&format!("... <truncated, {} bytes total>", bytes.len()));
        out
    }

    fn json_string(&self, v: &Value) -> String {
        if self.cfg.pretty {
            serde_json::to_string_pretty(v).unwrap_or_else(|_| "<unserializable>".into())
        } else {
            serde_json::to_string(v).unwrap_or_else(|_| "<unserializable>".into())
        }
    }
}

#[cfg(test)]
impl LogDumpMiddleware {
    pub(crate) fn render_left(&self, envelope: &RequestEnvelope<Value>) -> Option<String> {
        if !self.should_left() { return None; }
        // Clone before creating snapshot to avoid ownership issues
        let normalized_snapshot = envelope.normalized_snapshot
            .as_ref()
            .map(|s| self.redact_normalized_data(s.clone()));
        
        let req = &envelope.request_details;
        let backend_req = &envelope.backend_request_details;
        let tgt = envelope.target_details.clone();
        let normalized = self.redact_normalized_data(envelope.normalized_data.clone().unwrap_or(Value::Null));
        let snapshot = serde_json::json!({
            "side": "left",
            "label": if self.cfg.label.is_empty() { "dump" } else { &self.cfg.label },
            "request_details": {
                "method": req.method,
                "uri": req.uri,
                "headers": self.redact_headers(req.headers.clone()),
                "cookies": req.cookies,
                "query_params": req.query_params,
                "cache_status": req.cache_status,
                "metadata": self.redact_metadata(req.metadata.clone()),
                "content_metadata": req.content_metadata,
            },
            "backend_request_details": {
                "method": backend_req.method,
                "uri": backend_req.uri,
                "headers": self.redact_headers(backend_req.headers.clone()),
                "cookies": backend_req.cookies,
                "query_params": backend_req.query_params,
                "cache_status": backend_req.cache_status,
                "metadata": self.redact_metadata(backend_req.metadata.clone()),
                "content_metadata": backend_req.content_metadata,
            },
            "target_details": tgt.map(|mut t| {
                t.headers = self.redact_headers(t.headers);
                t.metadata = self.redact_metadata(t.metadata);
                serde_json::to_value(t).unwrap_or(Value::Null)
            }),
            "normalized_data": normalized,
            "normalized_snapshot": normalized_snapshot,
        });
        let s = self.json_string(&snapshot);
        Some(self.truncate_bytes(s))
    }

    pub(crate) fn render_right(&self, envelope: &ResponseEnvelope<Value>) -> Option<String> {
        if !self.should_right() { return None; }
        let req = &envelope.request_details;
        let res = &envelope.response_details;
        let normalized = self.redact_normalized_data(envelope.normalized_data.clone().unwrap_or(Value::Null));
        let normalized_snapshot = envelope.normalized_snapshot.as_ref()
            .map(|s| self.redact_normalized_data(s.clone()));
        let snapshot = serde_json::json!({
            "side": "right",
            "label": if self.cfg.label.is_empty() { "dump" } else { &self.cfg.label },
            "request_details": {
                "method": req.method,
                "uri": req.uri,
                "headers": self.redact_headers(req.headers.clone()),
                "cookies": req.cookies,
                "query_params": req.query_params,
                "cache_status": req.cache_status,
                "metadata": self.redact_metadata(req.metadata.clone()),
                "content_metadata": req.content_metadata,
            },
            "response_details": {
                "status": res.status,
                "headers": self.redact_headers(res.headers.clone()),
                "metadata": self.redact_metadata(res.metadata.clone()),
            },
            "normalized_data": normalized,
            "normalized_snapshot": normalized_snapshot,
        });
        let s = self.json_string(&snapshot);
        Some(self.truncate_bytes(s))
    }
}

#[async_trait]
impl Middleware for LogDumpMiddleware {
    async fn left(
        &self,
        envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<Value>, Error> {
        if !self.should_left() { return Ok(envelope); }

        let label = if self.cfg.label.is_empty() { "dump" } else { &self.cfg.label };

        // Clone before creating snapshot to avoid ownership issues
        let normalized_snapshot = envelope.normalized_snapshot
            .as_ref()
            .map(|s| self.redact_normalized_data(s.clone()));

        // Build a safe-to-log snapshot struct
        let req = &envelope.request_details;
        let backend_req = &envelope.backend_request_details;
        let tgt = envelope.target_details.clone();
        let normalized = self.redact_normalized_data(envelope.normalized_data.clone().unwrap_or(Value::Null));
        let snapshot = serde_json::json!({
            "side": "left",
            "label": label,
            "request_details": {
                "method": req.method,
                "uri": req.uri,
                "headers": self.redact_headers(req.headers.clone()),
                "cookies": req.cookies, // assumed non-sensitive session ids can be redacted by config
                "query_params": req.query_params,
                "cache_status": req.cache_status,
                "metadata": self.redact_metadata(req.metadata.clone()),
                "content_metadata": req.content_metadata,
            },
            "backend_request_details": {
                "method": backend_req.method,
                "uri": backend_req.uri,
                "headers": self.redact_headers(backend_req.headers.clone()),
                "cookies": backend_req.cookies,
                "query_params": backend_req.query_params,
                "cache_status": backend_req.cache_status,
                "metadata": self.redact_metadata(backend_req.metadata.clone()),
                "content_metadata": backend_req.content_metadata,
            },
            "target_details": tgt.map(|mut t| { 
                t.headers = self.redact_headers(t.headers);
                t.metadata = self.redact_metadata(t.metadata);
                serde_json::to_value(t).unwrap_or(Value::Null)
            }),
            "normalized_data": normalized,
            "normalized_snapshot": normalized_snapshot,
        });

        let serialized = self.json_string(&snapshot);
        let serialized = self.truncate_bytes(serialized);
        tracing::info!(target: "harmony.dump", "{}", serialized);
        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<Value>,
    ) -> Result<ResponseEnvelope<Value>, Error> {
        if !self.should_right() { return Ok(envelope); }

        let label = if self.cfg.label.is_empty() { "dump" } else { &self.cfg.label };
        
        // Clone before creating snapshot to avoid ownership issues
        let normalized_snapshot = envelope.normalized_snapshot
            .as_ref()
            .map(|s| self.redact_normalized_data(s.clone()));
            
        let req = &envelope.request_details;
        let res = &envelope.response_details;
        let normalized = self.redact_normalized_data(envelope.normalized_data.clone().unwrap_or(Value::Null));
        let snapshot = serde_json::json!({
            "side": "right",
            "label": label,
            "request_details": {
                "method": req.method,
                "uri": req.uri,
                "headers": self.redact_headers(req.headers.clone()),
                "cookies": req.cookies,
                "query_params": req.query_params,
                "cache_status": req.cache_status,
                "metadata": self.redact_metadata(req.metadata.clone()),
                "content_metadata": req.content_metadata,
            },
            "response_details": {
                "status": res.status,
                "headers": self.redact_headers(res.headers.clone()),
                "metadata": self.redact_metadata(res.metadata.clone()),
            },
            "normalized_data": normalized,
            "normalized_snapshot": normalized_snapshot,
        });

        let serialized = self.json_string(&snapshot);
        let serialized = self.truncate_bytes(serialized);
        tracing::info!(target: "harmony.dump", "{}", serialized);
        Ok(envelope)
    }
}

/// Parse configuration from HashMap for middleware registry
pub fn parse_config(options: &HashMap<String, Value>) -> Result<LogDumpMiddlewareConfig, String> {
    let apply = options
        .get("apply").and_then(|v| v.as_str()).unwrap_or("both").to_string();
    let pretty = options
        .get("pretty").and_then(|v| v.as_bool()).unwrap_or(true);
    let max_bytes = options
        .get("max_bytes").and_then(|v| v.as_u64()).map(|u| u as usize).unwrap_or(default_max_bytes());
    let redact_headers = options
        .get("redact_headers").and_then(|v| v.as_array()).map(|arr| {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        }).unwrap_or_default();
    let redact_metadata = options
        .get("redact_metadata").and_then(|v| v.as_array()).map(|arr| {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        }).unwrap_or_default();
    let redact_data_fields = options
        .get("redact_data_fields").and_then(|v| v.as_array()).map(|arr| {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        }).unwrap_or_default();
    let label = options
        .get("label").and_then(|v| v.as_str()).unwrap_or("").to_string();

    Ok(LogDumpMiddlewareConfig { apply, pretty, max_bytes, redact_headers, redact_metadata, redact_data_fields, label })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{RequestEnvelopeBuilder, ResponseDetails, ResponseEnvelope};
    use serde_json::json;

    #[tokio::test]
    async fn test_dump_left_and_right() {
        let cfg = LogDumpMiddlewareConfig { apply: "both".into(), pretty: false, max_bytes: 10_000, redact_headers: vec!["authorization".into()], redact_metadata: vec!["token".into()], redact_data_fields: vec![], label: "t1".into() };
        let mw = LogDumpMiddleware::new(cfg);

        let env = RequestEnvelopeBuilder::new()
            .method("GET").uri("/x").header("authorization", "Bearer ABC").metadata_entry("token", "XYZ")
            .original_data(serde_json::json!({"a":1}))
            .normalized_data(Some(serde_json::json!({"n":true})))
            .build().unwrap();

        let env = mw.left(env).await.unwrap();
        let resp = ResponseEnvelope { request_details: env.request_details.clone(), response_details: ResponseDetails { status: 200, headers: HashMap::new(), metadata: HashMap::new() }, original_data: serde_json::json!({}), normalized_data: env.normalized_data.clone(), normalized_snapshot: None };
        let _ = mw.right(resp).await.unwrap();
    }

    #[test]
    fn test_parse_config_with_redact_data_fields() {
        let mut options = HashMap::new();
        options.insert("apply".into(), json!("left"));
        options.insert("pretty".into(), json!(false));
        options.insert("max_bytes".into(), json!(1234));
        options.insert("redact_headers".into(), json!(["authorization"]));
        options.insert("redact_metadata".into(), json!(["token"]));
        options.insert("redact_data_fields".into(), json!(["patient.name", "ids[0]"]));
        options.insert("label".into(), json!("unit"));

        let cfg = parse_config(&options).unwrap();
        assert_eq!(cfg.apply, "left");
        assert!(!cfg.pretty);
        assert_eq!(cfg.max_bytes, 1234);
        assert_eq!(cfg.redact_headers, vec!["authorization"]);
        assert_eq!(cfg.redact_metadata, vec!["token"]);
        assert_eq!(cfg.redact_data_fields, vec!["patient.name", "ids[0]"]);
        assert_eq!(cfg.label, "unit");
    }

    #[test]
    fn test_redact_normalized_data_fields_nested_and_arrays() {
        let cfg = LogDumpMiddlewareConfig { 
            apply: "both".into(), 
            pretty: false, 
            max_bytes: 10_000, 
            redact_headers: vec![], 
            redact_metadata: vec![], 
            redact_data_fields: vec!["a.b".to_string(), "arr.secret".to_string()], 
            label: "".into() 
        };
        let mw = LogDumpMiddleware::new(cfg);
        let data = json!({
            "a": {"b": 123, "c": 5},
            "arr": [{"secret":"x"}, {"secret":"y", "keep":1}],
            "other": true
        });
        let red = mw.redact_normalized_data(data);
        assert_eq!(red["a"]["b"], json!("<redacted>"));
        assert_eq!(red["a"]["c"], json!(5));
        assert_eq!(red["arr"][0]["secret"], json!("<redacted>"));
        assert_eq!(red["arr"][1]["secret"], json!("<redacted>"));
        assert_eq!(red["arr"][1]["keep"], json!(1));
        assert_eq!(red["other"], json!(true));
    }

    #[test]
    fn test_header_and_metadata_redaction() {
        let cfg = LogDumpMiddlewareConfig { apply: "both".into(), pretty: false, max_bytes: 10_000, redact_headers: vec!["authorization".into()], redact_metadata: vec!["token".into()], redact_data_fields: vec![], label: "".into() };
        let mw = LogDumpMiddleware::new(cfg);

        let env = RequestEnvelopeBuilder::new()
            .method("GET").uri("/x")
            .header("Authorization", "Bearer abc")
            .header("x-other", "ok")
            .metadata_entry("token", "xyz")
            .metadata_entry("safe", "1")
            .original_data(json!({}))
            .normalized_data(Some(json!({"patient": {"name": "John"}})))
            .build().unwrap();

        let rendered = mw.render_left(&env).expect("should render");
        // authorization header redacted - check that value is <redacted>
        assert!(rendered.contains("\"Authorization\":\"<redacted>\""));
        assert!(rendered.contains("\"x-other\":\"ok\""));
        // metadata redacted
        assert!(rendered.contains("\"token\":\"<redacted>\""));
        assert!(rendered.contains("\"safe\":\"1\""));
    }

    #[test]
    fn test_apply_direction_controls_render() {
        let cfg_left = LogDumpMiddlewareConfig { apply: "left".into(), pretty: false, max_bytes: 10_000, redact_headers: vec![], redact_metadata: vec![], redact_data_fields: vec![], label: "".into() };
        let mw_left = LogDumpMiddleware::new(cfg_left);

        let env = RequestEnvelopeBuilder::new()
            .method("GET").uri("/x").original_data(json!({}))
            .normalized_data(Some(json!({})))
            .build().unwrap();
        assert!(mw_left.render_left(&env).is_some());

        let resp = ResponseEnvelope {
            request_details: env.request_details.clone(),
            response_details: ResponseDetails { status: 200, headers: HashMap::new(), metadata: HashMap::new() },
            original_data: json!({}),
            normalized_data: Some(json!({})),
            normalized_snapshot: None,
        };
        assert!(mw_left.render_right(&resp).is_none());
    }

    #[test]
    fn test_truncation_applies() {
        let cfg = LogDumpMiddlewareConfig { apply: "both".into(), pretty: false, max_bytes: 50, redact_headers: vec![], redact_metadata: vec![], redact_data_fields: vec![], label: "".into() };
        let mw = LogDumpMiddleware::new(cfg);
        let big = "x".repeat(10_000);
        let env = RequestEnvelopeBuilder::new()
            .method("GET").uri("/x").original_data(json!({}))
            .normalized_data(Some(json!({"big": big})))
            .build().unwrap();
        let s = mw.render_left(&env).unwrap();
        assert!(s.contains("<truncated"));
    }
}
