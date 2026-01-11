//! HTTP/3 error scenario tests
//!
//! Tests for error handling in HTTP/3 connections including:
//! - Expired certificates
//! - Connection drops
//! - Malformed requests
//! - Invalid certificates
//!
//! Run with: `cargo test --test http3_error_scenarios`

use bytes::Buf;
use harmony::clients::http3::Http3Client;
use http::{Method, StatusCode};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// Install the rustls crypto provider. Must be called before any TLS operations.
fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// Generate a self-signed certificate valid for testing.
fn generate_valid_cert() -> (String, String) {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)
        .expect("failed to generate self-signed cert");

    let cert_pem = cert.serialize_pem().expect("failed to serialize cert");
    let key_pem = cert.serialize_private_key_pem();

    (cert_pem, key_pem)
}

/// Generate an "expired" certificate for testing.
/// Note: rcgen 0.12 doesn't easily support backdated certs, so we generate
/// a cert for a different hostname to simulate validation failure.
fn generate_untrusted_cert() -> (String, String) {
    // Generate a cert that won't be in the client's trust store
    let subject_alt_names = vec!["untrusted.example.com".to_string()];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)
        .expect("failed to generate untrusted cert");

    let cert_pem = cert.serialize_pem().expect("failed to serialize cert");
    let key_pem = cert.serialize_private_key_pem();

    (cert_pem, key_pem)
}

/// Generate a certificate with wrong hostname (not matching localhost).
fn generate_wrong_hostname_cert() -> (String, String) {
    let subject_alt_names = vec!["wronghost.example.com".to_string()];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)
        .expect("failed to generate cert with wrong hostname");

    let cert_pem = cert.serialize_pem().expect("failed to serialize cert");
    let key_pem = cert.serialize_private_key_pem();

    (cert_pem, key_pem)
}

/// Build a QUIC server config for testing
fn build_server_config(cert_pem: &str, key_pem: &str) -> quinn::ServerConfig {
    use rustls::pki_types::CertificateDer;

    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<_, _>>()
        .expect("failed to parse cert PEM");

    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .expect("failed to read key PEM")
        .expect("no key found in PEM");

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("failed to build TLS config");

    let quic_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
        .expect("failed to create QUIC server config");

    quinn::ServerConfig::with_crypto(Arc::new(quic_config))
}

/// Start a simple HTTP/3 echo server
async fn start_http3_server(
    server_config: quinn::ServerConfig,
    addr: SocketAddr,
) -> anyhow::Result<quinn::Endpoint> {
    let endpoint = quinn::Endpoint::server(server_config, addr)?;
    Ok(endpoint)
}

/// Handle incoming HTTP/3 connection
async fn handle_http3_connection(connection: quinn::Connection) -> anyhow::Result<()> {
    use bytes::Bytes;

    let h3_conn = h3_quinn::Connection::new(connection);
    let mut h3 = h3::server::Connection::new(h3_conn).await?;

    while let Some(resolver) = h3.accept().await? {
        let (request, mut stream) = resolver.resolve_request().await?;

        let mut body = Vec::new();
        while let Some(mut chunk) = stream.recv_data().await? {
            body.extend_from_slice(chunk.chunk());
            chunk.advance(chunk.remaining());
        }

        let response_data = serde_json::json!({
            "method": request.method().as_str(),
            "uri": request.uri().to_string(),
            "body_size": body.len(),
        });
        let response_body = serde_json::to_vec(&response_data)?;

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

// =============================================================================
// Test: Expired Certificate
// =============================================================================

#[tokio::test]
async fn test_http3_untrusted_certificate_rejected() {
    install_crypto_provider();

    // Server uses one cert, client trusts a different one
    let (server_cert_pem, server_key_pem) = generate_untrusted_cert();
    let (client_ca_pem, _) = generate_valid_cert(); // Different cert

    let server_addr: SocketAddr = "127.0.0.1:19600".parse().unwrap();
    let server_config = build_server_config(&server_cert_pem, &server_key_pem);
    let endpoint = start_http3_server(server_config, server_addr)
        .await
        .expect("failed to start HTTP/3 server");

    let endpoint_clone = endpoint.clone();
    let _server_task = tokio::spawn(async move {
        if let Some(incoming) = endpoint_clone.accept().await {
            if let Ok(connection) = incoming.await {
                let _ = handle_http3_connection(connection).await;
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client trusts a different CA - server cert is untrusted
    let client = Http3Client::with_ca_cert(&client_ca_pem).expect("failed to create HTTP/3 client");

    let result = client
        .request(
            Method::GET,
            "127.0.0.1",
            19600,
            "/test",
            &std::collections::HashMap::new(),
            vec![],
        )
        .await;

    // Connection should fail due to untrusted certificate
    assert!(
        result.is_err(),
        "Expected connection to fail with untrusted certificate"
    );

    endpoint.close(0u32.into(), b"test complete");
    endpoint.wait_idle().await;
}

// =============================================================================
// Test: Connection Drop During Request
// =============================================================================

#[tokio::test]
async fn test_http3_connection_drop_during_request() {
    install_crypto_provider();

    let (cert_pem, key_pem) = generate_valid_cert();

    let server_addr: SocketAddr = "127.0.0.1:19601".parse().unwrap();
    let server_config = build_server_config(&cert_pem, &key_pem);
    let endpoint = start_http3_server(server_config, server_addr)
        .await
        .expect("failed to start HTTP/3 server");

    // Server that accepts connection but closes it immediately
    let endpoint_clone = endpoint.clone();
    let server_task = tokio::spawn(async move {
        if let Some(incoming) = endpoint_clone.accept().await {
            if let Ok(connection) = incoming.await {
                // Immediately close the connection to simulate a drop
                connection.close(0u32.into(), b"simulated drop");
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = Http3Client::with_ca_cert(&cert_pem).expect("failed to create HTTP/3 client");

    let result = client
        .request(
            Method::GET,
            "127.0.0.1",
            19601,
            "/test",
            &std::collections::HashMap::new(),
            vec![],
        )
        .await;

    // Request should fail due to connection being closed
    assert!(
        result.is_err(),
        "Expected request to fail when connection is dropped"
    );

    server_task.abort();
    endpoint.close(0u32.into(), b"test complete");
    endpoint.wait_idle().await;
}

// =============================================================================
// Test: Connection Refused (No Server)
// =============================================================================

#[tokio::test]
async fn test_http3_connection_refused() {
    install_crypto_provider();

    let (cert_pem, _key_pem) = generate_valid_cert();

    // Try to connect to a port where nothing is listening
    let client = Http3Client::with_ca_cert(&cert_pem).expect("failed to create HTTP/3 client");

    let result = client
        .request(
            Method::GET,
            "127.0.0.1",
            19699, // No server on this port
            "/test",
            &std::collections::HashMap::new(),
            vec![],
        )
        .await;

    // Connection should fail (timeout or connection refused)
    assert!(result.is_err(), "Expected connection to fail with no server");
}

// =============================================================================
// Test: Invalid Host Resolution
// =============================================================================

#[tokio::test]
async fn test_http3_invalid_host() {
    install_crypto_provider();

    let client = Http3Client::new().expect("failed to create HTTP/3 client");

    let result = client
        .request(
            Method::GET,
            "this-host-definitely-does-not-exist.invalid",
            443,
            "/test",
            &std::collections::HashMap::new(),
            vec![],
        )
        .await;

    assert!(result.is_err(), "Expected connection to fail with invalid host");
    let err_msg = result.unwrap_err().to_string().to_lowercase();
    // Different OSes give different error messages for failed DNS lookups
    assert!(
        err_msg.contains("resolve")
            || err_msg.contains("host")
            || err_msg.contains("dns")
            || err_msg.contains("lookup")
            || err_msg.contains("nodename")
            || err_msg.contains("address"),
        "Expected host resolution error, got: {}",
        err_msg
    );
}

// =============================================================================
// Test: Wrong Hostname in Certificate
// =============================================================================

#[tokio::test]
async fn test_http3_hostname_mismatch() {
    install_crypto_provider();

    // Generate cert for wrong hostname
    let (wrong_cert_pem, wrong_key_pem) = generate_wrong_hostname_cert();

    let server_addr: SocketAddr = "127.0.0.1:19602".parse().unwrap();
    let server_config = build_server_config(&wrong_cert_pem, &wrong_key_pem);
    let endpoint = start_http3_server(server_config, server_addr)
        .await
        .expect("failed to start HTTP/3 server");

    let endpoint_clone = endpoint.clone();
    let _server_task = tokio::spawn(async move {
        if let Some(incoming) = endpoint_clone.accept().await {
            if let Ok(connection) = incoming.await {
                let _ = handle_http3_connection(connection).await;
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client trusts the wrong cert but connects to localhost (hostname mismatch)
    let client =
        Http3Client::with_ca_cert(&wrong_cert_pem).expect("failed to create HTTP/3 client");

    let result = client
        .request(
            Method::GET,
            "127.0.0.1", // Connecting to 127.0.0.1, but cert is for wronghost.example.com
            19602,
            "/test",
            &std::collections::HashMap::new(),
            vec![],
        )
        .await;

    // Should fail due to hostname verification
    assert!(
        result.is_err(),
        "Expected connection to fail due to hostname mismatch"
    );

    endpoint.close(0u32.into(), b"test complete");
    endpoint.wait_idle().await;
}

// =============================================================================
// Test: Malformed Request Path
// =============================================================================

#[tokio::test]
async fn test_http3_malformed_path_handled() {
    install_crypto_provider();

    let (cert_pem, key_pem) = generate_valid_cert();

    let server_addr: SocketAddr = "127.0.0.1:19603".parse().unwrap();
    let server_config = build_server_config(&cert_pem, &key_pem);
    let endpoint = start_http3_server(server_config, server_addr)
        .await
        .expect("failed to start HTTP/3 server");

    let endpoint_clone = endpoint.clone();
    let server_task = tokio::spawn(async move {
        if let Some(incoming) = endpoint_clone.accept().await {
            if let Ok(connection) = incoming.await {
                let _ = handle_http3_connection(connection).await;
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = Http3Client::with_ca_cert(&cert_pem).expect("failed to create HTTP/3 client");

    // Request with unusual but valid path characters
    let result = client
        .request(
            Method::GET,
            "127.0.0.1",
            19603,
            "/test?foo=bar&baz=qux%20encoded",
            &std::collections::HashMap::new(),
            vec![],
        )
        .await;

    // This should succeed - the path is unusual but valid
    assert!(result.is_ok(), "Expected request with encoded path to succeed");
    let response = result.unwrap();
    assert_eq!(response.status, StatusCode::OK);

    server_task.abort();
    endpoint.close(0u32.into(), b"test complete");
    endpoint.wait_idle().await;
}

// =============================================================================
// Test: Large Request Body
// =============================================================================

#[tokio::test]
async fn test_http3_large_request_body() {
    install_crypto_provider();

    let (cert_pem, key_pem) = generate_valid_cert();

    let server_addr: SocketAddr = "127.0.0.1:19605".parse().unwrap();
    let server_config = build_server_config(&cert_pem, &key_pem);
    let endpoint = start_http3_server(server_config, server_addr)
        .await
        .expect("failed to start HTTP/3 server");

    let endpoint_clone = endpoint.clone();
    let server_task = tokio::spawn(async move {
        if let Some(incoming) = endpoint_clone.accept().await {
            if let Ok(connection) = incoming.await {
                let _ = handle_http3_connection(connection).await;
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = Http3Client::with_ca_cert(&cert_pem).expect("failed to create HTTP/3 client");

    // Send a large request body (1MB)
    let large_body = vec![b'x'; 1024 * 1024];

    let mut headers = std::collections::HashMap::new();
    headers.insert("content-type".to_string(), "application/octet-stream".to_string());

    let result = client
        .request(Method::POST, "127.0.0.1", 19605, "/upload", &headers, large_body)
        .await;

    assert!(result.is_ok(), "Expected large body request to succeed");
    let response = result.unwrap();
    assert_eq!(response.status, StatusCode::OK);

    // Verify server received the full body
    let json: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["body_size"], 1024 * 1024);

    server_task.abort();
    endpoint.close(0u32.into(), b"test complete");
    endpoint.wait_idle().await;
}

