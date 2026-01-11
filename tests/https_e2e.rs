use harmony::config::config::Config;
use harmony::run;
use rcgen::generate_simple_self_signed;
use reqwest::Client;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

/// Generate a self-signed certificate for testing
fn generate_test_certificate(temp_dir: &TempDir) -> (String, String) {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let cert = generate_simple_self_signed(subject_alt_names).expect("Failed to generate certificate");

    let cert_path = temp_dir.path().join("cert.pem");
    let key_path = temp_dir.path().join("key.pem");

    fs::write(&cert_path, cert.serialize_pem().expect("Failed to serialize cert"))
        .expect("Failed to write certificate");
    fs::write(&key_path, cert.serialize_private_key_pem()).expect("Failed to write key");

    (
        cert_path.to_string_lossy().to_string(),
        key_path.to_string_lossy().to_string(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn test_https_server_e2e() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let (cert_path, key_path) = generate_test_certificate(&temp_dir);

    let config_content = format!(
        r#"
[proxy]
id = "test-https-e2e"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "info"

[management]
enabled = false

[network.secure]
interface = "wg0"
enable_wireguard = false

[network.secure.http]
bind_address = "127.0.0.1"
bind_port = 18443
cert_path = "{}"
key_path = "{}"

[pipelines.test]
description = "Test HTTPS"
networks = ["secure"]
endpoints = ["echo_endpoint"]
backends = ["echo_backend"]
middleware = []

[endpoints.echo_endpoint]
service = "echo"

[backends.echo_backend]
service = "echo"

[services.echo]
module = ""
    "#,
        cert_path, key_path
    );

    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, &config_content).expect("Failed to write config");

    // Parse config
    let config: Config = toml::from_str(&config_content).expect("Failed to parse config");

    // Start the server in a background task
    let server_handle = tokio::spawn(async move {
        run(config).await;
    });

    // Give the server time to start
    sleep(Duration::from_millis(500)).await;

    // Create a client that accepts self-signed certificates
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("Failed to build client");

    // Make HTTPS request
    let response = client
        .get("https://127.0.0.1:18443/echo")
        .send()
        .await
        .expect("Failed to make HTTPS request");

    assert_eq!(response.status(), 200);
    
    // Cleanup
    server_handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_force_https_redirect_e2e() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let config_content = r#"
[proxy]
id = "test-force-https-e2e"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "info"

[management]
enabled = false

[network.redirect]
interface = "wg0"
enable_wireguard = false

[network.redirect.http]
bind_address = "127.0.0.1"
bind_port = 18080
force_https = true

[pipelines.test]
description = "Test redirect"
networks = ["redirect"]
endpoints = ["echo_endpoint"]
backends = ["echo_backend"]
middleware = []

[endpoints.echo_endpoint]
service = "echo"

[backends.echo_backend]
service = "echo"

[services.echo]
module = ""
    "#;

    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_content).expect("Failed to write config");

    // Parse config
    let config: Config = toml::from_str(config_content).expect("Failed to parse config");

    // Start the server in a background task
    let server_handle = tokio::spawn(async move {
        run(config).await;
    });

    // Give the server time to start
    sleep(Duration::from_millis(500)).await;

    // Create a client that doesn't follow redirects
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to build client");

    // Make HTTP request
    let response = client
        .get("http://127.0.0.1:18080/test/path?query=value")
        .send()
        .await
        .expect("Failed to make HTTP request");

    // Should get 301 redirect
    assert_eq!(response.status(), 301);
    
    // Check Location header points to HTTPS
    let location = response
        .headers()
        .get("location")
        .expect("No Location header")
        .to_str()
        .expect("Invalid Location header");
    
    assert!(location.starts_with("https://"));
    assert!(location.contains("/test/path?query=value"));
    
    // Cleanup
    server_handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_plain_http_e2e() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let config_content = r#"
[proxy]
id = "test-http-e2e"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "info"

[management]
enabled = false

[network.plain]
interface = "wg0"
enable_wireguard = false

[network.plain.http]
bind_address = "127.0.0.1"
bind_port = 18081

[pipelines.test]
description = "Test plain HTTP"
networks = ["plain"]
endpoints = ["echo_endpoint"]
backends = ["echo_backend"]
middleware = []

[endpoints.echo_endpoint]
service = "echo"

[backends.echo_backend]
service = "echo"

[services.echo]
module = ""
    "#;

    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_content).expect("Failed to write config");

    // Parse config
    let config: Config = toml::from_str(config_content).expect("Failed to parse config");

    // Start the server in a background task
    let server_handle = tokio::spawn(async move {
        run(config).await;
    });

    // Give the server time to start
    sleep(Duration::from_millis(500)).await;

    // Make plain HTTP request
    let client = Client::new();
    let response = client
        .get("http://127.0.0.1:18081/echo")
        .send()
        .await
        .expect("Failed to make HTTP request");

    assert_eq!(response.status(), 200);
    
    // Cleanup
    server_handle.abort();
}
