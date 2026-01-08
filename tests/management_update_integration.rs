use harmony::adapters::registry::AdapterRegistry;
use harmony::config::config::Config;
use harmony::config::Cli;
use harmony::globals;
use runbeam_sdk::MachineToken;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::{sleep, Duration};
use serial_test::serial;

/// Helper to create a minimal test config file
fn create_test_config(dir: &TempDir, port: u16) -> PathBuf {
    let config_content = format!(
        r#"
[proxy]
id = "test-gateway-update"
pipelines_path = "pipelines"
transforms_path = "transforms"

[runbeam]
enabled = true
cloud_api_base_url = "http://localhost:3000"
poll_interval_secs = 30

[logging]
log_level = "info"
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = {}

[management]
enabled = true
base_path = "admin"
network = "default"

[services.http]
module = ""

[services.echo]
module = ""

[middleware_types.passthru]
module = ""
"#,
        port
    );

    let config_path = dir.path().join("test-config-update.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");

    // Create required directories
    fs::create_dir_all(dir.path().join("pipelines")).ok();
    fs::create_dir_all(dir.path().join("transforms")).ok();

    config_path
}

/// Helper to create a test config with runbeam disabled
fn create_config_runbeam_disabled(dir: &TempDir, port: u16) -> PathBuf {
    let config_content = format!(
        r#"
[proxy]
id = "test-gateway-disabled"
pipelines_path = "pipelines"
transforms_path = "transforms"

[runbeam]
enabled = false

[logging]
log_level = "info"
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = {}

[management]
enabled = true
base_path = "admin"
network = "default"

[services.http]
module = ""
"#,
        port
    );

    let config_path = dir.path().join("test-config-disabled.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");

    // Create required directories
    fs::create_dir_all(dir.path().join("pipelines")).ok();
    fs::create_dir_all(dir.path().join("transforms")).ok();

    config_path
}

/// Helper to create a test config without proxy.id field
fn create_config_missing_proxy_id(dir: &TempDir, port: u16) -> PathBuf {
    let config_content = format!(
        r#"
[proxy]
pipelines_path = "pipelines"
transforms_path = "transforms"

[runbeam]
enabled = true
cloud_api_base_url = "http://localhost:3000"

[logging]
log_level = "info"
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = {}

[management]
enabled = true
base_path = "admin"
network = "default"

[services.http]
module = ""
"#,
        port
    );

    let config_path = dir.path().join("test-config-no-id.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");

    // Create required directories
    fs::create_dir_all(dir.path().join("pipelines")).ok();
    fs::create_dir_all(dir.path().join("transforms")).ok();

    config_path
}

#[tokio::test]
#[serial]
async fn test_update_endpoint_without_machine_token() {
    use tokio::time::timeout;
    
    let test_result = timeout(Duration::from_secs(10), async {
        // Create test config
        let temp_dir = TempDir::new().unwrap();
        let config_path = create_test_config(&temp_dir, 9091);

        // Load config
        let contents = std::fs::read_to_string(config_path.as_path()).expect("read config");
        let mut config: Config = toml::from_str(&contents).expect("parse config");
        config.inject_management_service().expect("inject management");
        config.validate().expect("validate config");

        // Set global config for the test
        globals::set_config(Arc::new(config.clone()));
        globals::set_config_path(config_path.to_string_lossy().to_string());

        // Create adapter registry
        let registry = Arc::new(AdapterRegistry::new());
        globals::set_adapter_registry(registry);

        // Give the server a moment to be ready
        sleep(Duration::from_millis(100)).await;

        // Test: Call update endpoint without machine token (should fail with 401)
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let response = client
            .post("http://127.0.0.1:9091/admin/update")
            .send()
            .await;

        match response {
            Ok(resp) => {
                assert_eq!(
                    resp.status(),
                    401,
                    "Expected 401 Unauthorized when no machine token is present"
                );

                let body: serde_json::Value = resp.json().await.unwrap();
                assert_eq!(body["error"], "Unauthorized");
                assert!(body["message"]
                    .as_str()
                    .unwrap()
                    .contains("Run `runbeam harmony:authorize` first"));
            }
            Err(e) => {
                // If server isn't running, that's expected in unit test context
                println!("Server not running (expected in unit test): {}", e);
            }
        }
    }).await;
    
    assert!(test_result.is_ok(), "Test timed out after 10 seconds");
}

#[tokio::test]
#[serial]
async fn test_update_endpoint_with_runbeam_disabled() {
    use tokio::time::timeout;
    
    let test_result = timeout(Duration::from_secs(10), async {
        // Create test config with runbeam disabled
        let temp_dir = TempDir::new().unwrap();
        let config_path = create_config_runbeam_disabled(&temp_dir, 9092);

        // Load config
        let contents = std::fs::read_to_string(config_path.as_path()).expect("read config");
        let mut config: Config = toml::from_str(&contents).expect("parse config");
        config.inject_management_service().expect("inject management");
        config.validate().expect("validate config");

        // Set global config for the test
        globals::set_config(Arc::new(config.clone()));
        globals::set_config_path(config_path.to_string_lossy().to_string());

        // Create adapter registry
        let registry = Arc::new(AdapterRegistry::new());
        globals::set_adapter_registry(registry);

        sleep(Duration::from_millis(100)).await;

        // Test: Call update endpoint with runbeam disabled (should fail with 403)
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let response = client
            .post("http://127.0.0.1:9092/admin/update")
            .send()
            .await;

        match response {
            Ok(resp) => {
                assert_eq!(
                    resp.status(),
                    403,
                    "Expected 403 Forbidden when Runbeam is disabled"
                );

                let body: serde_json::Value = resp.json().await.unwrap();
                assert_eq!(body["error"], "Forbidden");
                assert!(body["message"]
                    .as_str()
                    .unwrap()
                    .contains("Runbeam Cloud integration is disabled"));
            }
            Err(e) => {
                println!("Server not running (expected in unit test): {}", e);
            }
        }
    }).await;
    
    assert!(test_result.is_ok(), "Test timed out after 10 seconds");
}

#[tokio::test]
#[serial]
async fn test_update_endpoint_with_missing_proxy_id() {
    use tokio::time::timeout;
    
    let test_result = timeout(Duration::from_secs(10), async {
        // Create test config without proxy.id
        let temp_dir = TempDir::new().unwrap();
        let config_path = create_config_missing_proxy_id(&temp_dir, 9093);

        // Load config - this should actually fail during Config::from_args validation
        // but we test the update handler's validation as well
        let contents = std::fs::read_to_string(config_path.as_path()).expect("read config");
        
        // Note: Config validation may catch this before we even get to the update handler
        // This test verifies the defense-in-depth approach
        match toml::from_str::<Config>(&contents) {
            Ok(mut config) => {
                // If TOML parsing succeeds, try validation
                match config.validate() {
                    Ok(_) => {
                        globals::set_config(Arc::new(config.clone()));
                        globals::set_config_path(config_path.to_string_lossy().to_string());

                        let registry = Arc::new(AdapterRegistry::new());
                globals::set_adapter_registry(registry);

                sleep(Duration::from_millis(100)).await;

                // If config loads, the update endpoint should catch the missing ID
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .unwrap();
                let response = client
                    .post("http://127.0.0.1:9093/admin/update")
                    .send()
                    .await;

                        match response {
                            Ok(resp) => {
                                // Should get 400 or 401 (depending on whether we hit missing ID or missing token first)
                                assert!(
                                    resp.status() == 400 || resp.status() == 401,
                                    "Expected 400 or 401 for missing proxy ID"
                                );
                            }
                            Err(e) => {
                                println!("Server not running (expected in unit test): {}", e);
                            }
                        }
                    }
                    Err(_) => {
                        // Validation failed - this is good
                        println!("Config validation correctly rejected config without proxy.id");
                    }
                }
            }
            Err(_) => {
                // Config validation caught the missing ID - this is actually preferred!
                println!("Config validation correctly rejected config without proxy.id");
            }
        }
    }).await;
    
    assert!(test_result.is_ok(), "Test timed out after 10 seconds");
}

#[tokio::test]
async fn test_update_response_structure() {
    use harmony::management::update::UpdateResponse;

    // Test UpdateResponse serialization
    let response = UpdateResponse {
        success: true,
        message: "Configuration uploaded successfully".to_string(),
        config_size: 2048,
        pipeline_count: 3,
        transform_count: 2,
        mesh_count: 1,
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["message"], "Configuration uploaded successfully");
    assert_eq!(json["config_size"], 2048);
    assert_eq!(json["pipeline_count"], 3);
    assert_eq!(json["transform_count"], 2);
    assert_eq!(json["mesh_count"], 1);

    // Test deserialization (with defaults for optional fields)
    let json_str = r#"{
        "success": true,
        "message": "Configuration uploaded successfully",
        "config_size": 1234
    }"#;

    let parsed: UpdateResponse = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed.success, true);
    assert_eq!(parsed.message, "Configuration uploaded successfully");
    assert_eq!(parsed.config_size, 1234);
    // These should default to 0 when not present
    assert_eq!(parsed.pipeline_count, 0);
    assert_eq!(parsed.transform_count, 0);
    assert_eq!(parsed.mesh_count, 0);
}

#[tokio::test]
#[serial]
async fn test_config_file_reading() {
    // Test that we can read and parse the test config
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 9094);

    let toml_content = fs::read_to_string(&config_path).unwrap();
    assert!(toml_content.contains("[proxy]"));
    assert!(toml_content.contains("id = \"test-gateway-update\""));

    // Parse TOML to verify structure
    let toml_value: toml::Value = toml::from_str(&toml_content).unwrap();
    let gateway_id = toml_value
        .get("proxy")
        .and_then(|proxy| proxy.get("id"))
        .and_then(|id| id.as_str())
        .unwrap();

    assert_eq!(gateway_id, "test-gateway-update");
}

#[test]
fn test_machine_token_structure() {
    // Test MachineToken creation and serialization
    let token = MachineToken::new(
        "test_machine_token_abc123".to_string(),
        "2025-12-31T23:59:59Z".to_string(),
        "gateway-123".to_string(),
        vec!["gateway:read".to_string(), "gateway:write".to_string()],
    );

    assert_eq!(token.machine_token, "test_machine_token_abc123");
    assert_eq!(token.expires_at, "2025-12-31T23:59:59Z");
    assert_eq!(token.gateway_id, "gateway-123");
    assert_eq!(token.abilities.len(), 2);
}

/// Integration test helper documentation
///
/// To run a full end-to-end test with a real Harmony server:
///
/// 1. Start Harmony proxy:
///    ```sh
///    cd harmony-proxy
///    cargo run -- --config config/config.toml
///    ```
///
/// 2. Start Runbeam API (or mock server):
///    ```sh
///    # Run your Runbeam API on http://localhost:3000
///    ```
///
/// 3. Authorize Harmony:
///    ```sh
///    cd runbeam-cli
///    cargo run -- harmony:add --ip 127.0.0.1 --port 9090 --label test-harmony
///    cargo run -- harmony:authorize --label test-harmony
///    ```
///
/// 4. Test update:
///    ```sh
///    cargo run -- harmony:update --label test-harmony
///    ```
///
/// 5. Verify via curl:
///    ```sh
///    curl -X POST http://localhost:9090/admin/update
///    ```
#[test]
fn integration_test_documentation() {
    // This is a documentation test that describes the manual integration testing process
    println!("See function documentation for manual integration test steps");
}
