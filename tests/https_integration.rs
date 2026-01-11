use harmony::config::config::Config;
use rcgen::generate_simple_self_signed;
use std::fs;
use tempfile::TempDir;

/// Generate a self-signed certificate for testing
fn generate_test_certificate(temp_dir: &TempDir) -> (String, String) {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let cert = generate_simple_self_signed(subject_alt_names).expect("Failed to generate certificate");

    let cert_path = temp_dir.path().join("cert.pem");
    let key_path = temp_dir.path().join("key.pem");

    fs::write(&cert_path, cert.serialize_pem().expect("Failed to serialize cert")).expect("Failed to write certificate");
    fs::write(&key_path, cert.serialize_private_key_pem()).expect("Failed to write key");

    (
        cert_path.to_string_lossy().to_string(),
        key_path.to_string_lossy().to_string(),
    )
}

#[tokio::test]
async fn test_https_network_config_with_tls() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let (cert_path, key_path) = generate_test_certificate(&temp_dir);

    let config_content = format!(
        r#"
[proxy]
id = "test-https-proxy"
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
bind_port = 8443
cert_path = "{}"
key_path = "{}"

[pipelines.test_pipeline]
description = "Test HTTPS pipeline"
networks = ["secure"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
middleware = []

[endpoints.test_endpoint]
service = "echo"

[backends.test_backend]
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

    // Verify TLS configuration was parsed correctly
    let network = config
        .network
        .get("secure")
        .expect("Network 'secure' not found");
    let tcp_config = network
        .tcp_config
        .as_ref()
        .expect("TCP config not found");

    assert_eq!(tcp_config.bind_address, "127.0.0.1");
    assert_eq!(tcp_config.bind_port, 8443);
    assert_eq!(tcp_config.cert_path, Some(cert_path.clone()));
    assert_eq!(tcp_config.key_path, Some(key_path.clone()));
}

#[tokio::test]
async fn test_http_network_config_without_tls() {
    let config_content = r#"
[proxy]
id = "test-http-proxy"
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
bind_port = 8080

[pipelines.test_pipeline]
description = "Test HTTP pipeline"
networks = ["plain"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
middleware = []

[endpoints.test_endpoint]
service = "echo"

[backends.test_backend]
service = "echo"

[services.echo]
module = ""
    "#;

    // Parse config
    let config: Config = toml::from_str(config_content).expect("Failed to parse config");

    // Verify plain HTTP configuration (no TLS)
    let network = config
        .network
        .get("plain")
        .expect("Network 'plain' not found");
    let tcp_config = network
        .tcp_config
        .as_ref()
        .expect("TCP config not found");

    assert_eq!(tcp_config.bind_address, "127.0.0.1");
    assert_eq!(tcp_config.bind_port, 8080);
    assert_eq!(tcp_config.cert_path, None);
    assert_eq!(tcp_config.key_path, None);
}

#[tokio::test]
async fn test_mixed_http_and_https_networks() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let (cert_path, key_path) = generate_test_certificate(&temp_dir);

    let config_content = format!(
        r#"
[proxy]
id = "test-mixed-proxy"
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
bind_port = 8080

[network.secure]
interface = "wg0"
enable_wireguard = false

[network.secure.http]
bind_address = "127.0.0.1"
bind_port = 8443
cert_path = "{}"
key_path = "{}"

[pipelines.internal]
description = "Internal HTTP pipeline"
networks = ["plain"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
middleware = []

[pipelines.external]
description = "External HTTPS pipeline"
networks = ["secure"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
middleware = []

[endpoints.test_endpoint]
service = "echo"

[backends.test_backend]
service = "echo"

[services.echo]
module = ""
    "#,
        cert_path, key_path
    );

    // Parse config
    let config: Config = toml::from_str(&config_content).expect("Failed to parse config");

    // Verify plain HTTP network
    let plain_network = config
        .network
        .get("plain")
        .expect("Network 'plain' not found");
    let plain_tcp = plain_network
        .tcp_config
        .as_ref()
        .expect("Plain TCP config not found");

    assert_eq!(plain_tcp.bind_port, 8080);
    assert_eq!(plain_tcp.cert_path, None);
    assert_eq!(plain_tcp.key_path, None);

    // Verify HTTPS network
    let secure_network = config
        .network
        .get("secure")
        .expect("Network 'secure' not found");
    let secure_tcp = secure_network
        .tcp_config
        .as_ref()
        .expect("Secure TCP config not found");

    assert_eq!(secure_tcp.bind_port, 8443);
    assert_eq!(secure_tcp.cert_path, Some(cert_path));
    assert_eq!(secure_tcp.key_path, Some(key_path));
}

#[tokio::test]
async fn test_force_https_config() {
    let config_content = r#"
[proxy]
id = "test-force-https-proxy"
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
bind_port = 8080
force_https = true

[pipelines.test_pipeline]
description = "Test force_https pipeline"
networks = ["redirect"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
middleware = []

[endpoints.test_endpoint]
service = "echo"

[backends.test_backend]
service = "echo"

[services.echo]
module = ""
    "#;

    // Parse config
    let config: Config = toml::from_str(config_content).expect("Failed to parse config");

    // Verify force_https configuration
    let network = config
        .network
        .get("redirect")
        .expect("Network 'redirect' not found");
    let tcp_config = network
        .tcp_config
        .as_ref()
        .expect("TCP config not found");

    assert_eq!(tcp_config.bind_address, "127.0.0.1");
    assert_eq!(tcp_config.bind_port, 8080);
    assert_eq!(tcp_config.cert_path, None);
    assert_eq!(tcp_config.key_path, None);
    assert_eq!(tcp_config.force_https, true);
}
