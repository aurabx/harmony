//! HTTP/3 client for making outbound requests to upstream servers.
//!
//! This client uses `quinn` for QUIC transport and `h3` for HTTP/3 protocol handling.
//! It supports making requests to HTTP/3 servers from backend services.

use bytes::{Buf, Bytes};
use h3_quinn::Connection as H3QuinnConnection;
use http::{Method, Request, StatusCode};
use quinn::{ClientConfig, Endpoint};
use rustls::RootCertStore;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

/// Response from an HTTP/3 request.
#[derive(Debug)]
pub struct Http3Response {
    pub status: StatusCode,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

/// HTTP/3 client for making outbound requests.
///
/// This client creates a new QUIC connection for each request. Future versions
/// may implement connection pooling for better performance.
pub struct Http3Client {
    endpoint: Endpoint,
}

impl Http3Client {
    /// Create a new HTTP/3 client with default TLS configuration.
    ///
    /// Uses system root certificates for TLS validation.
    pub fn new() -> anyhow::Result<Self> {
        let client_config = Self::build_default_client_config()?;
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse::<SocketAddr>()?)?;
        endpoint.set_default_client_config(client_config);

        Ok(Self { endpoint })
    }

    /// Create a new HTTP/3 client with a custom CA certificate.
    ///
    /// This is useful for connecting to servers with self-signed certificates.
    pub fn with_ca_cert(ca_cert_pem: &str) -> anyhow::Result<Self> {
        let client_config = Self::build_client_config_with_ca(ca_cert_pem)?;
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse::<SocketAddr>()?)?;
        endpoint.set_default_client_config(client_config);

        Ok(Self { endpoint })
    }

    /// Build default client TLS config using system root certificates.
    fn build_default_client_config() -> anyhow::Result<ClientConfig> {
        // Use webpki-roots for system CA certificates
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();

        Ok(ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
        )))
    }

    /// Build client TLS config with a custom CA certificate.
    fn build_client_config_with_ca(ca_cert_pem: &str) -> anyhow::Result<ClientConfig> {
        use rustls::pki_types::CertificateDer;

        // Parse the PEM certificate
        let certs: Vec<CertificateDer<'static>> =
            rustls_pemfile::certs(&mut ca_cert_pem.as_bytes())
                .collect::<Result<_, _>>()?;

        if certs.is_empty() {
            anyhow::bail!("No certificates found in CA PEM");
        }

        // Create root cert store with custom CA
        let mut roots = RootCertStore::empty();
        for cert in certs {
            roots.add(cert)?;
        }

        let tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();

        Ok(ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
        )))
    }

    /// Make an HTTP/3 request to the specified URL.
    ///
    /// # Arguments
    /// * `method` - HTTP method (GET, POST, etc.)
    /// * `host` - Server hostname (used for SNI and Host header)
    /// * `port` - Server port
    /// * `path` - Request path (including query string)
    /// * `headers` - Additional request headers
    /// * `body` - Request body (empty for GET requests)
    pub async fn request(
        &self,
        method: Method,
        host: &str,
        port: u16,
        path: &str,
        headers: &HashMap<String, String>,
        body: Vec<u8>,
    ) -> anyhow::Result<Http3Response> {
        // Resolve host to socket address
        let addr = Self::resolve_host(host, port).await?;

        // Connect to server
        let connection = self
            .endpoint
            .connect(addr, host)?
            .await
            .map_err(|e| anyhow::anyhow!("QUIC connection failed: {}", e))?;

        // Create HTTP/3 connection
        let (mut driver, mut send_request) =
            h3::client::new(H3QuinnConnection::new(connection)).await?;

        // Spawn driver task to handle background control frames
        let driver_handle = tokio::spawn(async move {
            let _ = futures_util::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });

        // Build request
        let uri = format!("https://{}:{}{}", host, port, path);
        let mut req_builder = Request::builder()
            .method(method.clone())
            .uri(&uri);

        // Add headers
        for (key, value) in headers {
            req_builder = req_builder.header(key, value);
        }

        let req = req_builder.body(())?;

        // Send request
        let mut stream = send_request.send_request(req).await?;

        // Send body if present
        if !body.is_empty() {
            stream.send_data(Bytes::from(body)).await?;
        }
        stream.finish().await?;

        // Receive response
        let resp = stream.recv_response().await?;
        let status = resp.status();

        // Extract response headers
        let mut resp_headers = HashMap::new();
        for (key, value) in resp.headers() {
            if let Ok(value_str) = value.to_str() {
                resp_headers.insert(key.to_string(), value_str.to_string());
            }
        }

        // Read response body
        let mut resp_body = Vec::new();
        while let Some(mut chunk) = stream.recv_data().await? {
            resp_body.extend_from_slice(chunk.chunk());
            chunk.advance(chunk.remaining());
        }

        // Clean up
        drop(send_request);
        driver_handle.abort();

        Ok(Http3Response {
            status,
            headers: resp_headers,
            body: resp_body,
        })
    }

    /// Resolve hostname to socket address.
    async fn resolve_host(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
        use tokio::net::lookup_host;

        let addr_str = format!("{}:{}", host, port);
        let addrs: Vec<SocketAddr> = lookup_host(&addr_str).await?.collect();

        addrs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Failed to resolve host: {}", host))
    }

    /// Wait for the endpoint to become idle (all connections closed).
    pub async fn wait_idle(&self) {
        self.endpoint.wait_idle().await;
    }
}

impl Default for Http3Client {
    fn default() -> Self {
        Self::new().expect("Failed to create default Http3Client")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http3_response_debug() {
        let resp = Http3Response {
            status: StatusCode::OK,
            headers: HashMap::new(),
            body: vec![],
        };
        // Just ensure Debug is implemented
        let _ = format!("{:?}", resp);
    }

    #[tokio::test]
    async fn test_resolve_localhost() {
        let addr = Http3Client::resolve_host("127.0.0.1", 443).await;
        assert!(addr.is_ok());
        assert_eq!(addr.unwrap().port(), 443);
    }
}
