use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Standardized connection configuration shared across components.
///
/// # Protocol Support
///
/// The `protocol` field supports the following values:
/// - `"http"` - Plain HTTP (default if not specified)
/// - `"https"` - HTTP over TLS
/// - `"h3"` - HTTP/3 over QUIC (requires port 443 typically)
///
/// When using `h3` protocol, the connection will use QUIC transport with HTTP/3.
/// By default, system root CA certificates are used for TLS validation.
/// For self-signed certificates, provide a custom CA via `ca_cert_path`.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: Option<u16>,
    /// Protocol: "http", "https", or "h3" (HTTP/3 over QUIC)
    pub protocol: Option<String>,
    #[serde(default)]
    pub base_path: Option<String>,
    /// Path to custom CA certificate (PEM format) for TLS validation.
    /// Used with https and h3 protocols when connecting to servers with
    /// self-signed or custom CA certificates.
    #[serde(default)]
    pub ca_cert_path: Option<String>,
}

impl ConnectionConfig {
    /// Constructs a base URL from the connection configuration.
    ///
    /// For HTTP/3 connections (`protocol = "h3"`), returns an `https://` URL
    /// since HTTP/3 always uses TLS.
    pub fn to_base_url(&self) -> String {
        let protocol = self.protocol.as_deref().unwrap_or("http");
        // HTTP/3 uses https:// scheme in URLs
        let url_scheme = if protocol == "h3" { "https" } else { protocol };
        let port = self.port.map(|p| format!(":{}", p)).unwrap_or_default();
        let path = self.base_path.as_deref().unwrap_or("");
        let path = if !path.is_empty() && !path.starts_with('/') {
            format!("/{}", path)
        } else {
            path.to_string()
        };
        format!("{}://{}{}{}", url_scheme, self.host, port, path)
    }

    /// Returns true if this connection uses HTTP/3 (QUIC) protocol.
    pub fn is_http3(&self) -> bool {
        self.protocol.as_deref() == Some("h3")
    }
}

/// Global authentication definition (DSL v1.9.0+)
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct AuthenticationDefinition {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Reliability configuration (timeout, retries)
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ReliabilityConfig {
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_max_retries() -> u32 {
    3
}

impl Default for ReliabilityConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout_secs(),
            max_retries: default_max_retries(),
        }
    }
}
