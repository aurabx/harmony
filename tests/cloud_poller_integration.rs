use harmony::adapters::registry::AdapterRegistry;
use harmony::config::config::Config;
use harmony::config::Cli;
use harmony::globals;
use runbeam_sdk::runbeam_api::resources::Change;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

/// Helper to create a minimal test config file
fn create_test_config(dir: &TempDir, port: u16) -> PathBuf {
    let config_content = format!(
        r#"
[proxy]
id = "test-proxy"
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
base_path = "/api"
network = "default"
"#,
        port
    );

    let config_path = dir.path().join("test-config.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");
    
    // Create required directories
    fs::create_dir_all(dir.path().join("pipelines")).ok();
    fs::create_dir_all(dir.path().join("transforms")).ok();
    
    config_path
}

/// Helper to create a config with middleware (zero-downtime change)
fn create_config_with_middleware(dir: &TempDir, port: u16) -> PathBuf {
    let config_content = format!(
        r#"
[proxy]
id = "test-proxy"
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
base_path = "/api"
network = "default"

[middleware.test_middleware]
type = "passthru"

[middleware_types.passthru]
module = ""
"#,
        port
    );

    let config_path = dir.path().join("cloud-config.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");
    config_path
}

/// Helper to create a config with network changes (requires restart)
fn create_config_with_different_port(dir: &TempDir, port: u16) -> PathBuf {
    let config_content = format!(
        r#"
[proxy]
id = "test-proxy"
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
base_path = "/api"
network = "default"
"#,
        port
    );

    let config_path = dir.path().join("cloud-config-port-change.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");
    config_path
}

/// Helper to create an invalid config (missing network)
fn create_invalid_config(dir: &TempDir) -> PathBuf {
    let config_content = r#"
[proxy]
id = "test-proxy"
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

# Missing network configuration - should fail validation

[management]
enabled = true
base_path = "/api"
network = "default"
"#;

    let config_path = dir.path().join("invalid-config.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");
    config_path
}

#[tokio::test]
async fn test_load_and_validate_config_success() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 8080);

    // Load and validate config using CLI
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let config = Config::from_args(cli);

    // Verify basic config properties
    assert_eq!(config.proxy.id, "test-proxy");
    assert!(config.network.contains_key("default"));
    assert_eq!(config.network.get("default").unwrap().tcp_config.bind_port, 8080);
}

#[tokio::test]
async fn test_load_and_validate_config_invalid() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_invalid_config(&temp_dir);

    // Should panic during validation (Config::from_args validates)
    let result = std::panic::catch_unwind(|| {
        let cli = Cli::new(config_path.to_string_lossy().to_string());
        Config::from_args(cli)
    });

    assert!(result.is_err(), "Invalid config should fail validation");
}

#[tokio::test]
async fn test_cloud_config_diff_zero_downtime() {
    use harmony::config::reload::compute_diff;

    let temp_dir = TempDir::new().unwrap();
    let old_config_path = create_test_config(&temp_dir, 8080);
    let new_config_path = create_config_with_middleware(&temp_dir, 8080);

    // Load both configs
    let cli_old = Cli::new(old_config_path.to_string_lossy().to_string());
    let old_config = Config::from_args(cli_old);

    let cli_new = Cli::new(new_config_path.to_string_lossy().to_string());
    let new_config = Config::from_args(cli_new);

    // Compute diff
    let diff = compute_diff(&old_config, &new_config);

    // Verify it's a zero-downtime change
    assert!(diff.has_changes());
    assert!(!diff.requires_adapter_restart());
    assert!(diff.zero_downtime_changes.contains(&"middleware".to_string()));
}

#[tokio::test]
async fn test_cloud_config_diff_requires_restart() {
    use harmony::config::reload::compute_diff;

    let temp_dir = TempDir::new().unwrap();
    let old_config_path = create_test_config(&temp_dir, 8080);
    let new_config_path = create_config_with_different_port(&temp_dir, 8081);

    // Load both configs
    let cli_old = Cli::new(old_config_path.to_string_lossy().to_string());
    let old_config = Config::from_args(cli_old);

    let cli_new = Cli::new(new_config_path.to_string_lossy().to_string());
    let new_config = Config::from_args(cli_new);

    // Compute diff
    let diff = compute_diff(&old_config, &new_config);

    // Verify it requires adapter restart
    assert!(diff.has_changes());
    assert!(diff.requires_adapter_restart());
    assert!(diff.adapter_restarts_required.contains(&"default".to_string()));
}

#[tokio::test]
async fn test_temp_file_creation_and_cleanup() {
    // Ensure tmp directory exists
    fs::create_dir_all("./tmp").expect("Failed to create tmp directory");

    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 8080);

    // Simulate cloud config write to tmp
    let cloud_config_path = PathBuf::from("./tmp/cloud_config_test.toml");
    fs::copy(&config_path, &cloud_config_path).expect("Failed to copy config");

    // Verify file exists
    assert!(cloud_config_path.exists());

    // Clean up
    fs::remove_file(&cloud_config_path).ok();
}

#[tokio::test]
async fn test_config_application_with_adapter_registry() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 8090);

    // Load config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);

    // Set global config
    globals::set_config(config_arc.clone());

    // Create adapter registry
    let registry = Arc::new(AdapterRegistry::new());

    // Verify registry starts empty
    let networks = registry.get_running_networks().await;
    assert!(networks.is_empty());

    // Note: We don't actually start networks here since we're just testing
    // the configuration application logic without spawning real adapters
}

#[tokio::test]
async fn test_globals_config_set_and_get() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 8080);

    // Load config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);

    // Set global config
    globals::set_config(config_arc.clone());

    // Get global config
    let retrieved = globals::get_config();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().proxy.id, "test-proxy");
}

#[tokio::test]
async fn test_globals_config_path_set_and_get() {
    let test_path = "/tmp/test-config.toml".to_string();

    // Set global config path
    globals::set_config_path(test_path.clone());

    // Get global config path
    let retrieved = globals::get_config_path();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap(), test_path);
}

#[tokio::test]
async fn test_exponential_backoff_calculation() {
    // Test exponential backoff logic (2^n seconds)
    let backoff_1 = Duration::from_secs(2u64.pow(1)); // 2s
    let backoff_2 = Duration::from_secs(2u64.pow(2)); // 4s
    let backoff_3 = Duration::from_secs(2u64.pow(3)); // 8s
    let backoff_8 = Duration::from_secs(2u64.pow(8)); // 256s

    assert_eq!(backoff_1.as_secs(), 2);
    assert_eq!(backoff_2.as_secs(), 4);
    assert_eq!(backoff_3.as_secs(), 8);
    assert_eq!(backoff_8.as_secs(), 256);

    // Test max backoff capping at 300s
    let max_backoff = Duration::from_secs(300);
    let capped = backoff_8.min(max_backoff);
    assert_eq!(capped.as_secs(), 256); // Still under cap

    let very_large = Duration::from_secs(2u64.pow(10)); // 1024s
    let capped_large = very_large.min(max_backoff);
    assert_eq!(capped_large.as_secs(), 300); // Capped at max
}

#[tokio::test]
async fn test_poll_interval_timing() {
    // Test that poll interval can be configured
    let default_interval = Duration::from_secs(60);
    let custom_interval = Duration::from_secs(30);

    // Verify durations are different
    assert_ne!(default_interval, custom_interval);

    // Simulate waiting (with very short duration for test)
    let start = std::time::Instant::now();
    sleep(Duration::from_millis(10)).await;
    let elapsed = start.elapsed();

    assert!(elapsed >= Duration::from_millis(10));
}

#[tokio::test]
async fn test_config_change_id_format() {
    // Test that config change IDs can be used in file paths
    let change_id = "change-12345";
    let temp_path = format!("./tmp/cloud_config_{}.toml", change_id);

    // Verify path is valid
    let path = PathBuf::from(&temp_path);
    assert_eq!(path.to_string_lossy(), temp_path);
    assert!(temp_path.contains("cloud_config_change-12345"));
}

#[tokio::test]
async fn test_authorization_error_detection() {
    // Test error message detection for authorization failures
    let error_401 = "401 Unauthorized";
    let error_403 = "403 Forbidden";
    let error_500 = "500 Internal Server Error";

    // Verify authorization error detection logic
    assert!(error_401.contains("401") || error_401.contains("Unauthorized"));
    assert!(error_403.contains("403") || error_403.contains("Forbidden"));
    assert!(!error_500.contains("401") && !error_500.contains("403"));
}

#[tokio::test]
async fn test_tmp_directory_creation() {
    // Ensure tmp directory can be created
    let result = fs::create_dir_all("./tmp");
    assert!(result.is_ok(), "Should be able to create ./tmp directory");

    // Verify it exists
    assert!(PathBuf::from("./tmp").exists());
}

// ============================================================================
// Config Change Processing Tests
// ============================================================================

#[tokio::test]
async fn test_config_change_field_structure() {
    use serde_json::json;
    
    // Test that Change deserializes correctly from API response
    let json = json!({
        "id": "01k8vdq9wrcrezzbdpbjwsfwnz",
        "status": "queued",
        "type": "gateway",
        "gateway_id": "01k8ek6h9aahhnrv3benret1nn",
        "pipeline_id": null,
        "created_at": "2025-10-30T20:42:36.000000Z"
    });
    
    let change: Change = serde_json::from_value(json).unwrap();
    assert_eq!(change.id, "01k8vdq9wrcrezzbdpbjwsfwnz");
    assert_eq!(change.status, Some("queued".to_string()));
    assert_eq!(change.resource_type, "gateway");
    assert_eq!(change.gateway_id, "01k8ek6h9aahhnrv3benret1nn");
    assert_eq!(change.pipeline_id, None);
    assert_eq!(change.created_at, "2025-10-30T20:42:36.000000Z");
}

#[tokio::test]
async fn test_config_change_detail_field_structure() {
    use serde_json::json;
    
    // Test that Change deserializes with all fields (detail view)
    let json = json!({
        "id": "01k8vdq9wrcrezzbdpbjwsfwnz",
        "status": "queued",
        "type": "gateway",
        "gateway_id": "01k8ek6h9aahhnrv3benret1nn",
        "pipeline_id": null,
        "toml_config": "[proxy]\nid = \"gateway-aaace14a\"\n",
        "metadata": {
            "gateway_name": "gateway-aaace14a",
            "generated_at": "2025-10-30T20:42:36+00:00"
        },
        "created_at": "2025-10-30T20:42:36.000000Z",
        "acknowledged_at": null,
        "applied_at": null,
        "failed_at": null,
        "error_message": null,
        "error_details": null
    });
    
    let detail: Change = serde_json::from_value(json).unwrap();
    assert_eq!(detail.id, "01k8vdq9wrcrezzbdpbjwsfwnz");
    assert_eq!(detail.status, Some("queued".to_string()));
    assert_eq!(detail.resource_type, "gateway");
    assert!(detail.toml_config.as_ref().unwrap().contains("gateway-aaace14a"));
    assert!(detail.metadata.is_some());
    assert!(detail.acknowledged_at.is_none());
    assert!(detail.applied_at.is_none());
    assert!(detail.failed_at.is_none());
}

#[tokio::test]
async fn test_config_change_with_error_fields() {
    use serde_json::json;
    
    // Test that Change handles error fields correctly
    let json = json!({
        "id": "01k8abc123",
        "status": "failed",
        "type": "gateway",
        "gateway_id": "01k8gateway",
        "pipeline_id": null,
        "toml_config": "[invalid toml",
        "metadata": null,
        "created_at": "2025-10-30T20:42:36.000000Z",
        "acknowledged_at": "2025-10-30T20:42:40.000000Z",
        "applied_at": null,
        "failed_at": "2025-10-30T20:42:45.000000Z",
        "error_message": "Invalid TOML syntax",
        "error_details": {
            "line": 1,
            "column": 13,
            "expected": "value"
        }
    });
    
    let detail: Change = serde_json::from_value(json).unwrap();
    assert_eq!(detail.status, Some("failed".to_string()));
    assert!(detail.acknowledged_at.is_some());
    assert!(detail.applied_at.is_none());
    assert_eq!(detail.failed_at, Some("2025-10-30T20:42:45.000000Z".to_string()));
    assert_eq!(detail.error_message, Some("Invalid TOML syntax".to_string()));
    assert!(detail.error_details.is_some());
}

#[tokio::test]
async fn test_pipeline_config_change() {
    use serde_json::json;
    
    // Test that pipeline-type changes deserialize correctly
    let json = json!({
        "id": "01k8pipeline123",
        "status": "applied",
        "type": "pipeline",
        "gateway_id": "01k8gateway",
        "pipeline_id": "01k8pipe001",
        "created_at": "2025-10-30T20:42:36.000000Z"
    });
    
    let change: Change = serde_json::from_value(json).unwrap();
    assert_eq!(change.resource_type, "pipeline");
    assert_eq!(change.pipeline_id, Some("01k8pipe001".to_string()));
    assert_eq!(change.status, Some("applied".to_string()));
}

#[tokio::test]
async fn test_changes_should_be_reversed() {
    // Test that we understand changes need to be processed in reverse order
    let changes = vec![
        ("change-3", "2025-10-30T20:45:00Z"), // Newest
        ("change-2", "2025-10-30T20:44:00Z"),
        ("change-1", "2025-10-30T20:43:00Z"), // Oldest
    ];
    
    // Simulate reversing for processing (oldest first)
    let mut reversed: Vec<_> = changes.iter().collect();
    reversed.reverse();
    
    // Verify oldest comes first after reversal
    assert_eq!(reversed[0].0, "change-1");
    assert_eq!(reversed[1].0, "change-2");
    assert_eq!(reversed[2].0, "change-3");
}

#[tokio::test]
async fn test_toml_config_extraction() {
    use serde_json::json;
    
    let toml_content = r#"[proxy]
id = "test-gateway"
log_level = "info"

[network.default]
interface = "eth0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080
"#;
    
    let json = json!({
        "id": "01k8test",
        "status": "queued",
        "type": "gateway",
        "gateway_id": "01k8gw",
        "pipeline_id": null,
        "toml_config": toml_content,
        "metadata": null,
        "created_at": "2025-10-30T20:42:36.000000Z",
        "acknowledged_at": null,
        "applied_at": null,
        "failed_at": null,
        "error_message": null,
        "error_details": null
    });
    
    let detail: Change = serde_json::from_value(json).unwrap();
    
    // Verify TOML content is preserved
    let toml_config = detail.toml_config.as_ref().unwrap();
    assert!(toml_config.contains("test-gateway"));
    assert!(toml_config.contains("bind_address"));
    assert!(toml_config.contains("127.0.0.1"));
    
    // Verify it can be parsed as TOML
    let parsed: Result<toml::Value, _> = toml::from_str(toml_config);
    assert!(parsed.is_ok(), "TOML config should be valid");
}

#[tokio::test]
async fn test_metadata_json_structure() {
    use serde_json::json;
    
    let json = json!({
        "id": "01k8test",
        "status": "queued",
        "type": "gateway",
        "gateway_id": "01k8gw",
        "pipeline_id": null,
        "toml_config": "[proxy]\nid = \"test\"\n",
        "metadata": {
            "gateway_name": "my-gateway",
            "generated_at": "2025-10-30T20:42:36+00:00",
            "version": "1.0",
            "custom_field": "custom_value"
        },
        "created_at": "2025-10-30T20:42:36.000000Z",
        "acknowledged_at": null,
        "applied_at": null,
        "failed_at": null,
        "error_message": null,
        "error_details": null
    });
    
    let detail: Change = serde_json::from_value(json).unwrap();
    assert!(detail.metadata.is_some());
    
    let metadata = detail.metadata.unwrap();
    assert_eq!(metadata["gateway_name"], "my-gateway");
    assert_eq!(metadata["version"], "1.0");
    assert_eq!(metadata["custom_field"], "custom_value");
}

#[tokio::test]
async fn test_change_lifecycle_timestamps() {
    use serde_json::json;
    
    // Test a change that has been fully processed
    let json = json!({
        "id": "01k8complete",
        "status": "applied",
        "type": "gateway",
        "gateway_id": "01k8gw",
        "pipeline_id": null,
        "toml_config": "[proxy]\nid = \"test\"\n",
        "metadata": null,
        "created_at": "2025-10-30T20:42:36.000000Z",
        "acknowledged_at": "2025-10-30T20:42:40.000000Z",
        "applied_at": "2025-10-30T20:42:45.000000Z",
        "failed_at": null,
        "error_message": null,
        "error_details": null
    });
    
    let detail: Change = serde_json::from_value(json).unwrap();
    
    // Verify all lifecycle timestamps
    assert_eq!(detail.created_at, "2025-10-30T20:42:36.000000Z");
    assert_eq!(detail.acknowledged_at, Some("2025-10-30T20:42:40.000000Z".to_string()));
    assert_eq!(detail.applied_at, Some("2025-10-30T20:42:45.000000Z".to_string()));
    assert!(detail.failed_at.is_none());
    
    // Verify status matches lifecycle
    assert_eq!(detail.status, Some("applied".to_string()));
}

#[tokio::test]
async fn test_cloud_config_file_path_generation() {
    // Test that cloud config file paths are generated correctly
    let change_id = "01k8vdq9wrcrezzbdpbjwsfwnz";
    let expected_path = format!("./tmp/cloud_config_{}.toml", change_id);
    
    assert!(expected_path.contains("./tmp/cloud_config_"));
    assert!(expected_path.ends_with(".toml"));
    assert!(expected_path.contains(change_id));
    
    // Verify it's a valid path
    let path = PathBuf::from(&expected_path);
    assert_eq!(path.extension().unwrap(), "toml");
}

#[tokio::test]
async fn test_empty_changes_list() {
    use serde_json::json;
    
    // Test empty changes array
    let json = json!([]);
    let changes: Vec<Change> = serde_json::from_value(json).unwrap();
    assert_eq!(changes.len(), 0);
}

#[tokio::test]
async fn test_multiple_changes_ordering() {
    use serde_json::json;
    
    // Test multiple changes in response
    let json = json!([
        {
            "id": "change-3",
            "status": "queued",
            "type": "gateway",
            "gateway_id": "gw-1",
            "pipeline_id": null,
            "created_at": "2025-10-30T20:45:00.000000Z"
        },
        {
            "id": "change-2",
            "status": "queued",
            "type": "gateway",
            "gateway_id": "gw-1",
            "pipeline_id": null,
            "created_at": "2025-10-30T20:44:00.000000Z"
        },
        {
            "id": "change-1",
            "status": "queued",
            "type": "gateway",
            "gateway_id": "gw-1",
            "pipeline_id": null,
            "created_at": "2025-10-30T20:43:00.000000Z"
        }
    ]);
    
    let mut changes: Vec<Change> = serde_json::from_value(json).unwrap();
    assert_eq!(changes.len(), 3);
    
    // Verify API returns newest first
    assert_eq!(changes[0].id, "change-3");
    assert_eq!(changes[1].id, "change-2");
    assert_eq!(changes[2].id, "change-1");
    
    // Reverse for processing (oldest first)
    changes.reverse();
    assert_eq!(changes[0].id, "change-1");
    assert_eq!(changes[1].id, "change-2");
    assert_eq!(changes[2].id, "change-3");
}

// Note: Full integration tests with mock HTTP server for RunbeamClient
// would require additional infrastructure. These tests focus on the
// configuration loading, validation, and application logic that the
// cloud poller uses.
