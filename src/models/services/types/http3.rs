//! HTTP/3 backend service type.
//!
//! This service provides HTTP/3 (QUIC) backend connectivity, allowing Harmony
//! to proxy requests to upstream servers using HTTP/3 transport.
//!
//! # Configuration
//!
//! ```toml
//! [backends.my_h3_backend]
//! service = "http3"
//!
//! [backends.my_h3_backend.options]
//! host = "api.example.com"
//! port = 443
//! base_path = "/api/v1"
//! ca_cert_path = "/path/to/ca.pem"  # Optional: for self-signed certs
//! timeout_secs = 30                  # Optional: request timeout
//! ```
//!
//! Alternatively, use the connection config format:
//!
//! ```toml
//! [backends.my_h3_backend]
//! service = "http3"
//!
//! [backends.my_h3_backend.options.connection]
//! host = "api.example.com"
//! port = 443
//! base_path = "/api/v1"
//! ca_cert_path = "/path/to/ca.pem"
//! ```

use crate::clients::http3::Http3Client;
use crate::config::config::ConfigError;
use crate::models::connection::ConnectionConfig;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope, TargetDetails};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// HTTP/3 backend service for QUIC-based connections.
///
/// This backend service uses HTTP/3 over QUIC for all outbound connections,
/// providing benefits like:
/// - Multiplexed streams without head-of-line blocking
/// - 0-RTT connection resumption (future enhancement)
/// - Built-in TLS 1.3
#[derive(Debug, Deserialize, Default)]
pub struct Http3Backend {
    /// Optional pre-configured host (can be overridden by options)
    #[serde(default)]
    pub host: Option<String>,
    /// Optional pre-configured port (can be overridden by options)
    #[serde(default)]
    pub port: Option<u16>,
}

impl Http3Backend {
    /// Extract connection configuration from backend options.
    ///
    /// Supports both direct options (host, port, base_path) and
    /// nested connection config format.
    fn extract_connection_config(options: &HashMap<String, Value>) -> Option<ConnectionConfig> {
        // First try nested connection config
        if let Some(conn_json) = options.get("connection") {
            if let Ok(conn) = serde_json::from_value::<ConnectionConfig>(conn_json.clone()) {
                return Some(conn);
            }
        }

        // Fall back to direct options
        let host = options
            .get("host")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?;

        let port = options
            .get("port")
            .and_then(|v| v.as_u64())
            .map(|p| p as u16);

        let base_path = options
            .get("base_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let ca_cert_path = options
            .get("ca_cert_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Some(ConnectionConfig {
            host,
            port,
            protocol: Some("h3".to_string()),
            base_path,
            ca_cert_path,
        })
    }

    /// Build the base URL from connection configuration.
    fn build_base_url(conn: &ConnectionConfig) -> String {
        let port = conn.port.map(|p| format!(":{}", p)).unwrap_or_default();
        let path = conn.base_path.as_deref().unwrap_or("");
        let path = if !path.is_empty() && !path.starts_with('/') {
            format!("/{}", path)
        } else {
            path.to_string()
        };
        // HTTP/3 always uses https:// scheme
        format!("https://{}{}{}", conn.host, port, path)
    }
}

#[async_trait]
impl ServiceType for Http3Backend {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        let conn = Self::extract_connection_config(options).ok_or_else(|| {
            ConfigError::InvalidBackend {
                name: "http3".to_string(),
                reason: "HTTP/3 backend requires 'host' or 'connection.host' configuration"
                    .to_string(),
            }
        })?;

        // Validate host is not empty
        if conn.host.trim().is_empty() {
            return Err(ConfigError::InvalidBackend {
                name: "http3".to_string(),
                reason: "HTTP/3 backend 'host' cannot be empty".to_string(),
            });
        }

        // Validate port is reasonable if specified
        if let Some(port) = conn.port {
            if port == 0 {
                return Err(ConfigError::InvalidBackend {
                    name: "http3".to_string(),
                    reason: "HTTP/3 backend 'port' cannot be 0".to_string(),
                });
            }
        }

        // Validate CA cert path exists if specified
        if let Some(ref ca_path) = conn.ca_cert_path {
            if !ca_path.trim().is_empty() {
                let path = std::path::Path::new(ca_path);
                if !path.exists() {
                    return Err(ConfigError::InvalidBackend {
                        name: "http3".to_string(),
                        reason: format!(
                            "HTTP/3 backend ca_cert_path '{}' does not exist",
                            ca_path
                        ),
                    });
                }
                if !path.is_file() {
                    return Err(ConfigError::InvalidBackend {
                        name: "http3".to_string(),
                        reason: format!(
                            "HTTP/3 backend ca_cert_path '{}' is not a file",
                            ca_path
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // HTTP/3 backends don't define routes - they're targets for outbound requests
        vec![]
    }
}

#[async_trait]
impl ServiceHandler<Value> for Http3Backend {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // HTTP/3 backend doesn't handle incoming requests - it's a backend only
        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        let conn = Self::extract_connection_config(options).ok_or_else(|| {
            Error::from("HTTP/3 backend requires valid connection configuration")
        })?;

        let base_url = Self::build_base_url(&conn);

        // Build target details from request
        let target_details = if let Some(mut target) = envelope.target_details.take() {
            if target.base_url.is_empty() {
                target.base_url = base_url.clone();
            }
            target
        } else {
            let path = crate::models::services::path_utils::extract_path(&envelope);
            let mut target =
                TargetDetails::from_request_details(base_url.clone(), &envelope.request_details);
            target.uri = path;
            target
        };

        tracing::debug!(
            "HTTP/3 backend targeting: {} {}",
            target_details.method,
            target_details
                .full_url()
                .unwrap_or_else(|_| "<invalid-url>".to_string())
        );

        envelope.target_details = Some(target_details.clone());

        // Create HTTP/3 client
        let client = if let Some(ref ca_path) = conn.ca_cert_path {
            let ca_pem = std::fs::read_to_string(ca_path)
                .map_err(|e| Error::from(format!("Failed to read CA cert at {}: {}", ca_path, e)))?;
            Http3Client::with_ca_cert(&ca_pem)
                .map_err(|e| Error::from(format!("Failed to create HTTP/3 client with CA: {}", e)))?
        } else {
            Http3Client::new()
                .map_err(|e| Error::from(format!("Failed to create HTTP/3 client: {}", e)))?
        };

        let port = conn.port.unwrap_or(443);

        // Parse method
        let method = target_details
            .method
            .parse::<http::Method>()
            .map_err(|_| Error::from(format!("Invalid HTTP method: {}", target_details.method)))?;

        // Build path with query string
        let path = if target_details.query_params.is_empty() {
            target_details.uri.clone()
        } else {
            let query_string: String = target_details
                .query_params
                .iter()
                .flat_map(|(key, values)| {
                    values.iter().map(move |value| {
                        format!(
                            "{}={}",
                            urlencoding::encode(key),
                            urlencoding::encode(value)
                        )
                    })
                })
                .collect::<Vec<_>>()
                .join("&");
            format!("{}?{}", target_details.uri, query_string)
        };

        // Filter headers (drop hop-by-hop headers)
        let mut headers = HashMap::new();
        for (key, value) in &target_details.headers {
            let k = key.to_ascii_lowercase();
            if matches!(
                k.as_str(),
                "host"
                    | "connection"
                    | "keep-alive"
                    | "proxy-connection"
                    | "transfer-encoding"
                    | "upgrade"
                    | "content-length"
            ) {
                continue;
            }
            headers.insert(key.clone(), value.clone());
        }

        tracing::debug!(
            "Sending HTTP/3 request to: {}:{}{}",
            conn.host,
            port,
            path
        );

        let response = client
            .request(
                method,
                &conn.host,
                port,
                &path,
                &headers,
                envelope.original_data.clone(),
            )
            .await
            .map_err(|e| Error::from(format!("HTTP/3 request failed: {}", e)))?;

        let status = response.status.as_u16();
        tracing::debug!("HTTP/3 backend response status: {}", status);
        tracing::debug!(
            "HTTP/3 backend response body size: {} bytes",
            response.body.len()
        );

        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            response.headers,
            response.body,
            None,
        );

        // Try to parse response as JSON if content-type indicates JSON
        if let Some(content_type) = response_envelope
            .response_details
            .headers
            .get("content-type")
        {
            if content_type.contains("application/json") {
                if let Ok(json_value) =
                    serde_json::from_slice::<serde_json::Value>(&response_envelope.original_data)
                {
                    response_envelope.normalized_data = Some(json_value);
                }
            }
        }

        Ok(response_envelope)
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Build response from ResponseEnvelope
        let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        // Add headers, skip hop-by-hop headers
        for (k, v) in &envelope.response_details.headers {
            let key_lower = k.to_ascii_lowercase();
            if matches!(
                key_lower.as_str(),
                "content-length" | "transfer-encoding" | "connection" | "keep-alive"
            ) {
                continue;
            }
            builder = builder.header(k.as_str(), v.as_str());
        }

        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else if let Some(normalized) = envelope.normalized_data {
            let body_bytes = serde_json::to_vec(&normalized)
                .map_err(|_| Error::from("Failed to serialize HTTP/3 response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct HTTP/3 response"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_with_host() {
        let backend = Http3Backend::default();
        let mut options = HashMap::new();
        options.insert("host".to_string(), Value::String("example.com".to_string()));
        options.insert("port".to_string(), Value::Number(443.into()));

        assert!(backend.validate(&options).is_ok());
    }

    #[test]
    fn test_validate_missing_host() {
        let backend = Http3Backend::default();
        let options = HashMap::new();

        let result = backend.validate(&options);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_empty_host() {
        let backend = Http3Backend::default();
        let mut options = HashMap::new();
        options.insert("host".to_string(), Value::String("".to_string()));

        let result = backend.validate(&options);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let backend = Http3Backend::default();
        let mut options = HashMap::new();
        options.insert("host".to_string(), Value::String("example.com".to_string()));
        options.insert("port".to_string(), Value::Number(0.into()));

        let result = backend.validate(&options);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_with_connection_config() {
        let backend = Http3Backend::default();
        let mut options = HashMap::new();
        options.insert(
            "connection".to_string(),
            serde_json::json!({
                "host": "api.example.com",
                "port": 8443,
                "base_path": "/v1"
            }),
        );

        assert!(backend.validate(&options).is_ok());
    }

    #[test]
    fn test_extract_connection_direct() {
        let mut options = HashMap::new();
        options.insert("host".to_string(), Value::String("test.com".to_string()));
        options.insert("port".to_string(), Value::Number(443.into()));
        options.insert("base_path".to_string(), Value::String("/api".to_string()));

        let conn = Http3Backend::extract_connection_config(&options).unwrap();
        assert_eq!(conn.host, "test.com");
        assert_eq!(conn.port, Some(443));
        assert_eq!(conn.base_path, Some("/api".to_string()));
        assert_eq!(conn.protocol, Some("h3".to_string()));
    }

    #[test]
    fn test_extract_connection_nested() {
        let mut options = HashMap::new();
        options.insert(
            "connection".to_string(),
            serde_json::json!({
                "host": "nested.com",
                "port": 8443
            }),
        );

        let conn = Http3Backend::extract_connection_config(&options).unwrap();
        assert_eq!(conn.host, "nested.com");
        assert_eq!(conn.port, Some(8443));
    }

    #[test]
    fn test_build_base_url() {
        let conn = ConnectionConfig {
            host: "api.example.com".to_string(),
            port: Some(8443),
            protocol: Some("h3".to_string()),
            base_path: Some("/v1".to_string()),
            ca_cert_path: None,
        };

        let url = Http3Backend::build_base_url(&conn);
        assert_eq!(url, "https://api.example.com:8443/v1");
    }

    #[test]
    fn test_build_base_url_no_port() {
        let conn = ConnectionConfig {
            host: "api.example.com".to_string(),
            port: None,
            protocol: Some("h3".to_string()),
            base_path: None,
            ca_cert_path: None,
        };

        let url = Http3Backend::build_base_url(&conn);
        assert_eq!(url, "https://api.example.com");
    }

    #[test]
    fn test_build_router_returns_empty() {
        let backend = Http3Backend::default();
        let options = HashMap::new();
        let routes = backend.build_router(&options);
        assert!(routes.is_empty());
    }
}
