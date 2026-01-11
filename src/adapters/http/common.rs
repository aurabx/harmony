//! Common utilities shared between HTTP/1 and HTTP/3 adapters.
//!
//! This module provides a protocol-agnostic helper for building `ProtocolCtx`
//! from HTTP request components (method, URI, headers, body).

use crate::models::protocol::{Protocol, ProtocolCtx};
use http::{HeaderMap, Method, Uri};
use std::collections::HashMap;

/// Build a `ProtocolCtx` from HTTP request components.
///
/// This is a shared helper that can be used by both HTTP/1 (Axum) and HTTP/3 (h3)
/// adapters to construct a consistent `ProtocolCtx` from request data.
///
/// # Arguments
/// * `method` - HTTP method
/// * `uri` - Request URI
/// * `headers` - Request headers
/// * `body` - Request body bytes
/// * `path_prefix` - Optional path prefix to strip (from endpoint options)
/// * `protocol_label` - Protocol label for metadata (e.g., "http" or "http3")
pub fn build_protocol_ctx(
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Vec<u8>,
    path_prefix: &str,
    protocol_label: &str,
) -> ProtocolCtx {
    let path_only = uri.path().to_string();

    // Clean trailing semicolons from query strings (some clients/middleware add them)
    let full_path_with_query = uri
        .path_and_query()
        .map(|pq| pq.as_str().trim_end_matches(';').to_string())
        .unwrap_or_else(|| path_only.clone());

    // Strip prefix from path and remove leading slash
    let mut subpath = path_only
        .strip_prefix(path_prefix)
        .unwrap_or(&path_only)
        .to_string();
    if subpath.starts_with('/') {
        subpath = subpath.trim_start_matches('/').to_string();
    }

    // Also create subpath with query string (stripped prefix but includes query)
    let subpath_with_query = full_path_with_query
        .strip_prefix(path_prefix)
        .unwrap_or(&full_path_with_query)
        .trim_end_matches(';')
        .to_string();

    // Headers JSON object
    let headers_obj: serde_json::Value = {
        let map: serde_json::Map<String, serde_json::Value> = headers
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    serde_json::Value::String(v.to_str().unwrap_or_default().to_string()),
                )
            })
            .collect();
        serde_json::Value::Object(map)
    };

    // Cookies JSON object
    let cookies_obj: serde_json::Value = {
        let mut map = serde_json::Map::new();
        for val in headers.get_all(http::header::COOKIE).iter() {
            if let Ok(s) = val.to_str() {
                for part in s.split(';') {
                    let kv = part.trim();
                    if kv.is_empty() {
                        continue;
                    }
                    let mut split = kv.splitn(2, '=');
                    let name = split.next().unwrap_or("").trim();
                    let value = split.next().unwrap_or("").trim();
                    if !name.is_empty() {
                        map.insert(
                            name.to_string(),
                            serde_json::Value::String(value.to_string()),
                        );
                    }
                }
            }
        }
        serde_json::Value::Object(map)
    };

    // Query params JSON object
    // Note: Semicolons in query strings can be alternative separators (RFC 3986)
    // or trailing artifacts from some clients. We strip trailing semicolons from values.
    let query_obj: serde_json::Value = {
        let mut root = serde_json::Map::new();
        if let Some(q) = uri.query() {
            for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
                let clean_value = v.trim_end_matches(';').to_string();
                root.entry(k.to_string())
                    .or_insert_with(|| serde_json::Value::Array(vec![]))
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::Value::String(clean_value));
            }
        }
        serde_json::Value::Object(root)
    };

    // Cache status
    let cache_status = headers
        .get("Cache-Status")
        .or_else(|| headers.get("X-Cache"))
        .or_else(|| headers.get("CF-Cache-Status"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();

    // Metadata
    let mut meta_map = HashMap::new();
    meta_map.insert("protocol".to_string(), protocol_label.to_string());
    meta_map.insert("path".to_string(), subpath);
    meta_map.insert("path_with_query".to_string(), subpath_with_query);
    meta_map.insert("full_path".to_string(), full_path_with_query);

    // attrs object
    let mut attrs = serde_json::Map::new();
    attrs.insert(
        "method".to_string(),
        serde_json::Value::String(method.to_string()),
    );
    attrs.insert(
        "uri".to_string(),
        serde_json::Value::String(uri.to_string()),
    );
    attrs.insert("headers".to_string(), headers_obj);
    attrs.insert("cookies".to_string(), cookies_obj);
    attrs.insert("query_params".to_string(), query_obj);
    attrs.insert(
        "cache_status".to_string(),
        serde_json::Value::String(cache_status),
    );

    ProtocolCtx {
        protocol: Protocol::Http,
        payload: body,
        meta: meta_map,
        attrs: serde_json::Value::Object(attrs),
    }
}

/// Extract path_prefix from endpoint options.
pub fn get_path_prefix(options: &HashMap<String, serde_json::Value>) -> &str {
    options
        .get("path_prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::COOKIE;

    #[test]
    fn test_build_protocol_ctx_basic() {
        let method = Method::GET;
        let uri: Uri = "/api/test?foo=bar".parse().unwrap();
        let headers = HeaderMap::new();
        let body = vec![];

        let ctx = build_protocol_ctx(&method, &uri, &headers, body, "", "http");

        assert_eq!(ctx.protocol, Protocol::Http);
        assert_eq!(ctx.meta.get("protocol").unwrap(), "http");
        assert_eq!(ctx.meta.get("path").unwrap(), "api/test");
        assert_eq!(ctx.meta.get("full_path").unwrap(), "/api/test?foo=bar");
        assert_eq!(ctx.attrs["method"], "GET");
    }

    #[test]
    fn test_build_protocol_ctx_with_path_prefix() {
        let method = Method::POST;
        let uri: Uri = "/api/v1/users".parse().unwrap();
        let headers = HeaderMap::new();
        let body = b"hello".to_vec();

        let ctx = build_protocol_ctx(&method, &uri, &headers, body.clone(), "/api/v1", "http3");

        assert_eq!(ctx.meta.get("protocol").unwrap(), "http3");
        assert_eq!(ctx.meta.get("path").unwrap(), "users");
        assert_eq!(ctx.payload, body);
    }

    #[test]
    fn test_build_protocol_ctx_cookies() {
        let method = Method::GET;
        let uri: Uri = "/test".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, "session=abc123; user=john".parse().unwrap());
        let body = vec![];

        let ctx = build_protocol_ctx(&method, &uri, &headers, body, "", "http");

        let cookies = &ctx.attrs["cookies"];
        assert_eq!(cookies["session"], "abc123");
        assert_eq!(cookies["user"], "john");
    }

    #[test]
    fn test_build_protocol_ctx_query_params() {
        let method = Method::GET;
        let uri: Uri = "/search?q=test&page=1&page=2".parse().unwrap();
        let headers = HeaderMap::new();
        let body = vec![];

        let ctx = build_protocol_ctx(&method, &uri, &headers, body, "", "http");

        let query = &ctx.attrs["query_params"];
        assert!(query["q"].is_array());
        assert_eq!(query["q"][0], "test");
        // Multiple values for same key
        assert!(query["page"].is_array());
    }

    #[test]
    fn test_get_path_prefix() {
        let mut options = HashMap::new();
        assert_eq!(get_path_prefix(&options), "");

        options.insert(
            "path_prefix".to_string(),
            serde_json::Value::String("/api".to_string()),
        );
        assert_eq!(get_path_prefix(&options), "/api");
    }
}
