//! End-to-end HTTP/3 test
//!
//! This test spins up Harmony with both HTTP and HTTP/3 listeners, generates
//! self-signed test certificates, and uses quinn + h3 client APIs to perform
//! real HTTP/3 requests.
//!
//! The test is marked #[ignore] by default since QUIC handshake timing can be
//! flaky in CI. Run it locally with: `cargo test http3_e2e -- --ignored`

use bytes::Buf;
use harmony::adapters::registry::AdapterRegistry;
use harmony::config::config::Config;
use harmony::config::Cli;
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

/// Generate a self-signed certificate and private key for testing.
/// Returns (cert_pem, key_pem) as strings.
fn generate_self_signed_cert() -> (String, String) {
    // Use rcgen to generate a self-signed cert
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)
        .expect("failed to generate self-signed cert");

    let cert_pem = cert.serialize_pem().expect("failed to serialize cert");
    let key_pem = cert.serialize_private_key_pem();

    (cert_pem, key_pem)
}

/// Create test config with HTTP and HTTP/3 listeners
fn create_http3_test_config(
    dir: &TempDir,
    http_port: u16,
    http3_port: u16,
    cert_path: &str,
    key_path: &str,
) -> String {
    let config_content = format!(
        r#"
[proxy]
id = "http3-test-proxy"
log_level = "debug"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[network.test]
interface = "lo0"
enable_wireguard = false

[network.test.http]
bind_address = "127.0.0.1"
bind_port = {}

[network.test.http3]
bind_address = "127.0.0.1"
bind_port = {}
cert_path = "{}"
key_path = "{}"

[pipelines.echo_pipeline]
description = "Echo pipeline for HTTP/3 testing"
networks = ["test"]
endpoints = ["http_endpoint"]
backends = ["echo_backend"]
middleware = []

[endpoints.http_endpoint]
service = "http"
[endpoints.http_endpoint.options]
path_prefix = "/api"

[backends.echo_backend]
service = "echo"

[services.http]
module = ""

[services.echo]
module = ""

[management]
enabled = false
"#,
        http_port, http3_port, cert_path, key_path
    );

    let config_path = dir.path().join("http3-test-config.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");

    // Create required directories
    fs::create_dir_all(dir.path().join("pipelines")).ok();
    fs::create_dir_all(dir.path().join("transforms")).ok();
    fs::create_dir_all(dir.path().join("tmp")).ok();

    config_path.to_string_lossy().to_string()
}

/// Build a quinn client config that trusts our self-signed cert
fn build_client_config(cert_pem: &str) -> quinn::ClientConfig {
    use rustls::pki_types::CertificateDer;
    use rustls::RootCertStore;

    // Parse the PEM certificate
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<_, _>>()
        .expect("failed to parse cert PEM");

    // Create root cert store with our self-signed cert
    let mut roots = RootCertStore::empty();
    for cert in &certs {
        roots.add(cert.clone()).expect("failed to add cert to roots");
    }

    // Build rustls client config
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    // Build quinn client config
    quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
            .expect("failed to create QUIC client config"),
    ))
}

/// Perform an HTTP/3 GET request using quinn + h3
async fn http3_get(
    client_config: quinn::ClientConfig,
    addr: SocketAddr,
    path: &str,
) -> anyhow::Result<(u16, Vec<u8>)> {
    use http::{Method, Request, Version};

    // Create QUIC endpoint
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    // Connect to server
    let connection = endpoint.connect(addr, "localhost")?.await?;

    // Create HTTP/3 connection
    let (mut driver, mut send_request) = h3::client::new(h3_quinn::Connection::new(connection)).await?;

    // Spawn driver task - poll_close returns ConnectionError directly on close
    let driver_handle = tokio::spawn(async move {
        let _ = futures_util::future::poll_fn(|cx| driver.poll_close(cx)).await;
    });

    // Build request
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("https://localhost{}", path))
        .version(Version::HTTP_3)
        .body(())?;

    // Send request
    let mut stream = send_request.send_request(req).await?;
    stream.finish().await?;

    // Receive response
    let resp = stream.recv_response().await?;
    let status = resp.status().as_u16();

    // Read body
    let mut body = Vec::new();
    while let Some(mut chunk) = stream.recv_data().await? {
        body.extend_from_slice(chunk.chunk());
        chunk.advance(chunk.remaining());
    }

    // Clean up
    drop(send_request);
    driver_handle.abort();
    endpoint.wait_idle().await;

    Ok((status, body))
}

/// Perform an HTTP/3 POST request with a body
async fn http3_post(
    client_config: quinn::ClientConfig,
    addr: SocketAddr,
    path: &str,
    body: &[u8],
) -> anyhow::Result<(u16, Vec<u8>)> {
    use bytes::Bytes;
    use http::{Method, Request, Version};

    // Create QUIC endpoint
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    // Connect to server
    let connection = endpoint.connect(addr, "localhost")?.await?;

    // Create HTTP/3 connection
    let (mut driver, mut send_request) = h3::client::new(h3_quinn::Connection::new(connection)).await?;

    // Spawn driver task - poll_close returns ConnectionError directly on close
    let driver_handle = tokio::spawn(async move {
        let _ = futures_util::future::poll_fn(|cx| driver.poll_close(cx)).await;
    });

    // Build request
    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("https://localhost{}", path))
        .version(Version::HTTP_3)
        .header("content-type", "application/json")
        .body(())?;

    // Send request with body
    let mut stream = send_request.send_request(req).await?;
    stream.send_data(Bytes::copy_from_slice(body)).await?;
    stream.finish().await?;

    // Receive response
    let resp = stream.recv_response().await?;
    let status = resp.status().as_u16();

    // Read response body
    let mut resp_body = Vec::new();
    while let Some(mut chunk) = stream.recv_data().await? {
        resp_body.extend_from_slice(chunk.chunk());
        chunk.advance(chunk.remaining());
    }

    // Clean up
    drop(send_request);
    driver_handle.abort();
    endpoint.wait_idle().await;

    Ok((status, resp_body))
}

#[tokio::test]
#[ignore] // Run with: cargo test http3_e2e -- --ignored
async fn test_http3_get_request() {
    // Generate self-signed cert
    let (cert_pem, key_pem) = generate_self_signed_cert();

    // Create temp directory and write cert/key
    let temp_dir = TempDir::new().unwrap();
    let cert_path = temp_dir.path().join("cert.pem");
    let key_path = temp_dir.path().join("key.pem");
    fs::write(&cert_path, &cert_pem).unwrap();
    fs::write(&key_path, &key_pem).unwrap();

    // Create config
    let http_port = 19080;
    let http3_port = 19443;
    let config_path = create_http3_test_config(
        &temp_dir,
        http_port,
        http3_port,
        cert_path.to_str().unwrap(),
        key_path.to_str().unwrap(),
    );

    // Load config and start network
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);

    let registry = Arc::new(AdapterRegistry::new());
    registry
        .start_network("test".to_string(), config_arc.clone())
        .await
        .expect("failed to start network");

    // Give adapters time to bind
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Build client config
    let client_config = build_client_config(&cert_pem);

    // Perform HTTP/3 GET request
    let addr: SocketAddr = format!("127.0.0.1:{}", http3_port).parse().unwrap();
    let result = http3_get(client_config, addr, "/api/ping").await;

    // Stop network
    registry.stop_all().await.ok();

    // Assert result
    let (status, body) = result.expect("HTTP/3 request failed");
    assert_eq!(status, 200, "Expected 200 OK");

    // Parse response body (echo backend returns JSON)
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response should be JSON");
    assert_eq!(json["path"], "ping");
    assert_eq!(json["full_path"], "/api/ping");
}

#[tokio::test]
#[ignore] // Run with: cargo test http3_e2e -- --ignored
async fn test_http3_post_request_with_body() {
    // Generate self-signed cert
    let (cert_pem, key_pem) = generate_self_signed_cert();

    // Create temp directory and write cert/key
    let temp_dir = TempDir::new().unwrap();
    let cert_path = temp_dir.path().join("cert.pem");
    let key_path = temp_dir.path().join("key.pem");
    fs::write(&cert_path, &cert_pem).unwrap();
    fs::write(&key_path, &key_pem).unwrap();

    // Create config
    let http_port = 19081;
    let http3_port = 19444;
    let config_path = create_http3_test_config(
        &temp_dir,
        http_port,
        http3_port,
        cert_path.to_str().unwrap(),
        key_path.to_str().unwrap(),
    );

    // Load config and start network
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);

    let registry = Arc::new(AdapterRegistry::new());
    registry
        .start_network("test".to_string(), config_arc.clone())
        .await
        .expect("failed to start network");

    // Give adapters time to bind
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Build client config
    let client_config = build_client_config(&cert_pem);

    // Prepare POST body
    let post_body = serde_json::json!({"message": "hello http3"});
    let body_bytes = serde_json::to_vec(&post_body).unwrap();

    // Perform HTTP/3 POST request
    let addr: SocketAddr = format!("127.0.0.1:{}", http3_port).parse().unwrap();
    let result = http3_post(client_config, addr, "/api/echo", &body_bytes).await;

    // Stop network
    registry.stop_all().await.ok();

    // Assert result
    let (status, resp_body) = result.expect("HTTP/3 POST request failed");
    assert_eq!(status, 200, "Expected 200 OK");

    // Parse response body
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("response should be JSON");
    assert_eq!(json["path"], "echo");
    assert_eq!(json["full_path"], "/api/echo");
    // The echo backend should return information about the request
    assert!(json["headers"].is_object());
}

#[tokio::test]
#[ignore] // Run with: cargo test http3_e2e -- --ignored
async fn test_http3_not_found() {
    // Generate self-signed cert
    let (cert_pem, key_pem) = generate_self_signed_cert();

    // Create temp directory and write cert/key
    let temp_dir = TempDir::new().unwrap();
    let cert_path = temp_dir.path().join("cert.pem");
    let key_path = temp_dir.path().join("key.pem");
    fs::write(&cert_path, &cert_pem).unwrap();
    fs::write(&key_path, &key_pem).unwrap();

    // Create config
    let http_port = 19082;
    let http3_port = 19445;
    let config_path = create_http3_test_config(
        &temp_dir,
        http_port,
        http3_port,
        cert_path.to_str().unwrap(),
        key_path.to_str().unwrap(),
    );

    // Load config and start network
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);

    let registry = Arc::new(AdapterRegistry::new());
    registry
        .start_network("test".to_string(), config_arc.clone())
        .await
        .expect("failed to start network");

    // Give adapters time to bind
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Build client config
    let client_config = build_client_config(&cert_pem);

    // Perform HTTP/3 GET request to non-existent path
    let addr: SocketAddr = format!("127.0.0.1:{}", http3_port).parse().unwrap();
    let result = http3_get(client_config, addr, "/nonexistent/path").await;

    // Stop network
    registry.stop_all().await.ok();

    // Assert result - should get 404
    let (status, _body) = result.expect("HTTP/3 request failed");
    assert_eq!(status, 404, "Expected 404 Not Found");
}
