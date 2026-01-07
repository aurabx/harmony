//! End-to-end HTTP/3 backend test
//!
//! This test spins up an HTTP/3 server (using quinn + h3), configures Harmony
//! with an HTTP/3 backend, and verifies that outbound HTTP/3 connections work.
//!
//! The test is marked #[ignore] by default since QUIC handshake timing can be
//! flaky in CI. Run it locally with: `cargo test http3_backend_e2e -- --ignored`

use harmony::clients::http3::Http3Client;
use http::{Method, StatusCode};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// Install the rustls crypto provider. Must be called before any TLS operations.
fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// Generate a self-signed certificate and private key for testing.
/// Returns (cert_pem, key_pem) as strings.
fn generate_self_signed_cert() -> (String, String) {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)
        .expect("failed to generate self-signed cert");

    let cert_pem = cert.serialize_pem().expect("failed to serialize cert");
    let key_pem = cert.serialize_private_key_pem();

    (cert_pem, key_pem)
}

/// Build a QUIC server config for testing
fn build_server_config(cert_pem: &str, key_pem: &str) -> quinn::ServerConfig {
    use rustls::pki_types::CertificateDer;

    // Parse certificate
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<_, _>>()
        .expect("failed to parse cert PEM");

    // Parse private key
    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .expect("failed to read key PEM")
        .expect("no key found in PEM");

    // Build rustls server config
    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("failed to build TLS config");

    // Build quinn server config
    let quic_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
        .expect("failed to create QUIC server config");

    quinn::ServerConfig::with_crypto(Arc::new(quic_config))
}

/// Start a simple HTTP/3 echo server that returns request info as JSON
async fn start_http3_server(
    server_config: quinn::ServerConfig,
    addr: SocketAddr,
) -> anyhow::Result<quinn::Endpoint> {
    let endpoint = quinn::Endpoint::server(server_config, addr)?;
    Ok(endpoint)
}

/// Handle incoming HTTP/3 connections
async fn handle_http3_connection(connection: quinn::Connection) -> anyhow::Result<()> {
    use bytes::{Buf, Bytes};

    let h3_conn = h3_quinn::Connection::new(connection);
    let mut h3 = h3::server::Connection::new(h3_conn).await?;

    // Accept request stream
    while let Some(resolver) = h3.accept().await? {
        // Resolve the request
        let (request, mut stream) = resolver.resolve_request().await?;

        // Read request body
        let mut body = Vec::new();
        while let Some(mut chunk) = stream.recv_data().await? {
            body.extend_from_slice(chunk.chunk());
            chunk.advance(chunk.remaining());
        }

        // Build response JSON
        let response_data = serde_json::json!({
            "method": request.method().as_str(),
            "uri": request.uri().to_string(),
            "path": request.uri().path(),
            "headers": request.headers().iter().map(|(k, v)| {
                (k.to_string(), v.to_str().unwrap_or("").to_string())
            }).collect::<std::collections::HashMap<String, String>>(),
            "body_size": body.len(),
            "body": String::from_utf8_lossy(&body).to_string()
        });
        let response_body = serde_json::to_vec(&response_data)?;

        // Send response
        let response = http::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(())?;

        stream.send_response(response).await?;
        stream.send_data(Bytes::from(response_body)).await?;
        stream.finish().await?;
    }

    Ok(())
}

#[tokio::test]
async fn test_http3_client_get_request() {
    install_crypto_provider();

    // Generate self-signed cert
    let (cert_pem, key_pem) = generate_self_signed_cert();

    // Start HTTP/3 server
    let server_addr: SocketAddr = "127.0.0.1:19550".parse().unwrap();
    let server_config = build_server_config(&cert_pem, &key_pem);
    let endpoint = start_http3_server(server_config, server_addr)
        .await
        .expect("failed to start HTTP/3 server");

    // Spawn server handler
    let endpoint_clone = endpoint.clone();
    let server_task = tokio::spawn(async move {
        if let Some(incoming) = endpoint_clone.accept().await {
            if let Ok(connection) = incoming.await {
                let _ = handle_http3_connection(connection).await;
            }
        }
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create HTTP/3 client with custom CA
    let client = Http3Client::with_ca_cert(&cert_pem).expect("failed to create HTTP/3 client");

    // Make request
    let response = client
        .request(
            Method::GET,
            "127.0.0.1",
            19550,
            "/api/test?foo=bar",
            &std::collections::HashMap::new(),
            vec![],
        )
        .await
        .expect("HTTP/3 request failed");

    // Verify response
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.headers.contains_key("content-type"));

    // Parse response body
    let json: serde_json::Value =
        serde_json::from_slice(&response.body).expect("response should be JSON");
    assert_eq!(json["method"], "GET");
    assert_eq!(json["path"], "/api/test");
    assert!(json["uri"].as_str().unwrap().contains("foo=bar"));

    // Clean up
    server_task.abort();
    endpoint.close(0u32.into(), b"test complete");
    endpoint.wait_idle().await;
}

#[tokio::test]
async fn test_http3_client_post_request() {
    install_crypto_provider();

    // Generate self-signed cert
    let (cert_pem, key_pem) = generate_self_signed_cert();

    // Start HTTP/3 server
    let server_addr: SocketAddr = "127.0.0.1:19551".parse().unwrap();
    let server_config = build_server_config(&cert_pem, &key_pem);
    let endpoint = start_http3_server(server_config, server_addr)
        .await
        .expect("failed to start HTTP/3 server");

    // Spawn server handler
    let endpoint_clone = endpoint.clone();
    let server_task = tokio::spawn(async move {
        if let Some(incoming) = endpoint_clone.accept().await {
            if let Ok(connection) = incoming.await {
                let _ = handle_http3_connection(connection).await;
            }
        }
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create HTTP/3 client with custom CA
    let client = Http3Client::with_ca_cert(&cert_pem).expect("failed to create HTTP/3 client");

    // Prepare POST body
    let request_body = serde_json::json!({"message": "hello http3 backend"});
    let body_bytes = serde_json::to_vec(&request_body).unwrap();

    // Add content-type header
    let mut headers = std::collections::HashMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());

    // Make request
    let response = client
        .request(
            Method::POST,
            "127.0.0.1",
            19551,
            "/api/echo",
            &headers,
            body_bytes,
        )
        .await
        .expect("HTTP/3 POST request failed");

    // Verify response
    assert_eq!(response.status, StatusCode::OK);

    // Parse response body
    let json: serde_json::Value =
        serde_json::from_slice(&response.body).expect("response should be JSON");
    assert_eq!(json["method"], "POST");
    assert_eq!(json["path"], "/api/echo");
    assert!(json["body"].as_str().unwrap().contains("hello http3 backend"));

    // Clean up
    server_task.abort();
    endpoint.close(0u32.into(), b"test complete");
    endpoint.wait_idle().await;
}

#[tokio::test]
async fn test_http3_client_with_headers() {
    install_crypto_provider();

    // Generate self-signed cert
    let (cert_pem, key_pem) = generate_self_signed_cert();

    // Start HTTP/3 server
    let server_addr: SocketAddr = "127.0.0.1:19552".parse().unwrap();
    let server_config = build_server_config(&cert_pem, &key_pem);
    let endpoint = start_http3_server(server_config, server_addr)
        .await
        .expect("failed to start HTTP/3 server");

    // Spawn server handler
    let endpoint_clone = endpoint.clone();
    let server_task = tokio::spawn(async move {
        if let Some(incoming) = endpoint_clone.accept().await {
            if let Ok(connection) = incoming.await {
                let _ = handle_http3_connection(connection).await;
            }
        }
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create HTTP/3 client with custom CA
    let client = Http3Client::with_ca_cert(&cert_pem).expect("failed to create HTTP/3 client");

    // Add custom headers
    let mut headers = std::collections::HashMap::new();
    headers.insert("x-custom-header".to_string(), "custom-value".to_string());
    headers.insert("authorization".to_string(), "Bearer test-token".to_string());

    // Make request
    let response = client
        .request(
            Method::GET,
            "127.0.0.1",
            19552,
            "/api/headers",
            &headers,
            vec![],
        )
        .await
        .expect("HTTP/3 request with headers failed");

    // Verify response
    assert_eq!(response.status, StatusCode::OK);

    // Parse response body and verify headers were received
    let json: serde_json::Value =
        serde_json::from_slice(&response.body).expect("response should be JSON");
    let resp_headers = &json["headers"];
    assert_eq!(resp_headers["x-custom-header"], "custom-value");
    assert_eq!(resp_headers["authorization"], "Bearer test-token");

    // Clean up
    server_task.abort();
    endpoint.close(0u32.into(), b"test complete");
    endpoint.wait_idle().await;
}
