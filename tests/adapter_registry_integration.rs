use harmony::adapters::registry::AdapterRegistry;
use harmony::config::config::Config;
use harmony::config::Cli;
use harmony::globals;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::{timeout};

/// Helper to create a minimal test config with specified port
fn create_test_config(dir: &TempDir, network_name: &str, port: u16) -> String {
    let config_content = format!(
        r#"
[proxy]
id = "test-proxy"
log_level = "info"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[network.{}]
interface = "lo0"
enable_wireguard = false

[network.{}.http]
bind_address = "127.0.0.1"
bind_port = {}

[management]
enabled = false
"#,
        network_name, network_name, port
    );

    let config_path = dir.path().join(format!("config-{}.toml", network_name));
    fs::write(&config_path, config_content).expect("Failed to write test config");
    
    // Create required directories
    fs::create_dir_all(dir.path().join("pipelines")).ok();
    fs::create_dir_all(dir.path().join("transforms")).ok();
    
    config_path.to_string_lossy().to_string()
}

/// Helper to create config with multiple networks
fn create_multi_network_config(dir: &TempDir, ports: Vec<(&str, u16)>) -> String {
    let mut networks = String::new();
    for (name, port) in &ports {
        networks.push_str(&format!(
            r#"
[network.{}]
interface = "lo0"
enable_wireguard = false

[network.{}.http]
bind_address = "127.0.0.1"
bind_port = {}
"#,
            name, name, port
        ));
    }

    let config_content = format!(
        r#"
[proxy]
id = "test-proxy"
log_level = "info"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

{}

[management]
enabled = false
"#,
        networks
    );

    let config_path = dir.path().join("multi-network-config.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");
    
    // Create required directories
    fs::create_dir_all(dir.path().join("pipelines")).ok();
    fs::create_dir_all(dir.path().join("transforms")).ok();
    
    config_path.to_string_lossy().to_string()
}

#[tokio::test]
async fn test_registry_creation() {
    let registry = AdapterRegistry::new();
    let networks = registry.get_running_networks().await;
    assert!(networks.is_empty(), "New registry should have no running networks");
}

#[tokio::test]
async fn test_registry_default() {
    let registry = AdapterRegistry::default();
    let networks = registry.get_running_networks().await;
    assert!(networks.is_empty(), "Default registry should have no running networks");
}

#[tokio::test]
async fn test_start_network_success() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, "test_network", 18080);
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Start network
    let result = registry.start_network("test_network".to_string(), config_arc.clone()).await;
    assert!(result.is_ok(), "Should successfully start network");
    
    // Verify network is running
    let networks = registry.get_running_networks().await;
    assert_eq!(networks.len(), 1, "Should have 1 running network");
    assert!(networks.contains(&"test_network".to_string()));
    
    // Stop network
    registry.stop_network("test_network").await.ok();
}

#[tokio::test]
async fn test_start_network_invalid() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, "valid_network", 18081);
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Try to start network that doesn't exist in config
    let result = registry.start_network("nonexistent_network".to_string(), config_arc).await;
    assert!(result.is_err(), "Should fail to start non-existent network");
    
    // Verify no networks are running
    let networks = registry.get_running_networks().await;
    assert!(networks.is_empty(), "Should have no running networks after failure");
}

#[tokio::test]
async fn test_stop_network_success() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, "stop_test", 18082);
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Start network
    registry.start_network("stop_test".to_string(), config_arc).await.ok();
    
    // Verify it's running
    let networks = registry.get_running_networks().await;
    assert_eq!(networks.len(), 1);
    
    // Stop network
    let result = registry.stop_network("stop_test").await;
    assert!(result.is_ok(), "Should successfully stop network");
    
    // Verify it's stopped
    let networks = registry.get_running_networks().await;
    assert!(networks.is_empty(), "Should have no running networks after stop");
}

#[tokio::test]
async fn test_stop_network_nonexistent() {
    let registry = Arc::new(AdapterRegistry::new());
    
    // Try to stop network that was never started
    let result = registry.stop_network("nonexistent").await;
    assert!(result.is_ok(), "Stopping non-existent network should be no-op");
}

#[tokio::test]
async fn test_restart_network_success() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, "restart_test", 18083);
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Start network
    registry.start_network("restart_test".to_string(), config_arc.clone()).await.ok();
    
    // Restart network
    let result = registry.restart_network("restart_test".to_string(), config_arc).await;
    assert!(result.is_ok(), "Should successfully restart network");
    
    // Verify it's still running
    let networks = registry.get_running_networks().await;
    assert_eq!(networks.len(), 1);
    assert!(networks.contains(&"restart_test".to_string()));
    
    // Clean up
    registry.stop_network("restart_test").await.ok();
}

#[tokio::test]
async fn test_stop_all_with_multiple_networks() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_multi_network_config(
        &temp_dir,
        vec![("network1", 18084), ("network2", 18085), ("network3", 18086)]
    );
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Start multiple networks
    registry.start_network("network1".to_string(), config_arc.clone()).await.ok();
    registry.start_network("network2".to_string(), config_arc.clone()).await.ok();
    registry.start_network("network3".to_string(), config_arc.clone()).await.ok();
    
    // Verify all are running
    let networks = registry.get_running_networks().await;
    assert_eq!(networks.len(), 3, "Should have 3 running networks");
    
    // Stop all
    let result = registry.stop_all().await;
    assert!(result.is_ok(), "Should successfully stop all networks");
    
    // Verify all are stopped
    let networks = registry.get_running_networks().await;
    assert!(networks.is_empty(), "Should have no running networks after stop_all");
}

#[tokio::test]
async fn test_stop_all_empty_registry() {
    let registry = Arc::new(AdapterRegistry::new());
    
    // Stop all on empty registry
    let result = registry.stop_all().await;
    assert!(result.is_ok(), "Stopping empty registry should be no-op");
}

#[tokio::test]
async fn test_get_running_networks() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_multi_network_config(
        &temp_dir,
        vec![("net_a", 18087), ("net_b", 18088)]
    );
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Initially empty
    assert_eq!(registry.get_running_networks().await.len(), 0);
    
    // Start first network
    registry.start_network("net_a".to_string(), config_arc.clone()).await.ok();
    assert_eq!(registry.get_running_networks().await.len(), 1);
    
    // Start second network
    registry.start_network("net_b".to_string(), config_arc.clone()).await.ok();
    assert_eq!(registry.get_running_networks().await.len(), 2);
    
    // Stop first network
    registry.stop_network("net_a").await.ok();
    let networks = registry.get_running_networks().await;
    assert_eq!(networks.len(), 1);
    assert!(networks.contains(&"net_b".to_string()));
    
    // Clean up
    registry.stop_all().await.ok();
}

#[tokio::test]
async fn test_concurrent_start_operations() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_multi_network_config(
        &temp_dir,
        vec![("concurrent1", 18089), ("concurrent2", 18090)]
    );
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Start networks concurrently
    let registry1 = registry.clone();
    let config1 = config_arc.clone();
    let handle1 = tokio::spawn(async move {
        registry1.start_network("concurrent1".to_string(), config1).await
    });
    
    let registry2 = registry.clone();
    let config2 = config_arc.clone();
    let handle2 = tokio::spawn(async move {
        registry2.start_network("concurrent2".to_string(), config2).await
    });
    
    // Wait for both to complete
    let result1 = handle1.await.unwrap();
    let result2 = handle2.await.unwrap();
    
    assert!(result1.is_ok(), "First concurrent start should succeed");
    assert!(result2.is_ok(), "Second concurrent start should succeed");
    
    // Verify both are running
    let networks = registry.get_running_networks().await;
    assert_eq!(networks.len(), 2, "Should have 2 running networks");
    
    // Clean up
    registry.stop_all().await.ok();
}

#[tokio::test]
async fn test_adapter_shutdown_timeout() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, "timeout_test", 18091);
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Start network
    registry.start_network("timeout_test".to_string(), config_arc).await.ok();
    
    // Stop with timeout to ensure shutdown completes
    let stop_result = timeout(Duration::from_secs(5), registry.stop_network("timeout_test")).await;
    
    assert!(stop_result.is_ok(), "Stop should complete within timeout");
    assert!(stop_result.unwrap().is_ok(), "Stop should succeed");
}

#[tokio::test]
async fn test_network_lifecycle_state_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, "lifecycle_test", 18092);
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Initial state: empty
    assert_eq!(registry.get_running_networks().await.len(), 0);
    
    // Start network
    registry.start_network("lifecycle_test".to_string(), config_arc.clone()).await.ok();
    assert_eq!(registry.get_running_networks().await.len(), 1);
    
    // Restart network (should still have 1)
    registry.restart_network("lifecycle_test".to_string(), config_arc).await.ok();
    assert_eq!(registry.get_running_networks().await.len(), 1);
    
    // Stop network
    registry.stop_network("lifecycle_test").await.ok();
    assert_eq!(registry.get_running_networks().await.len(), 0);
}

#[tokio::test]
async fn test_registry_with_globals() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, "globals_test", 18093);
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    // Set global config
    globals::set_config(config_arc.clone());
    
    // Create registry
    let registry = Arc::new(AdapterRegistry::new());
    
    // Set global registry
    globals::set_adapter_registry(registry.clone());
    
    // Verify we can retrieve it (this tests the global state integration)
    let retrieved_config = globals::get_config();
    assert!(retrieved_config.is_some());
    assert_eq!(retrieved_config.unwrap().proxy.id, "test-proxy");
}

#[tokio::test]
async fn test_repeated_start_same_network() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, "repeated_test", 18094);
    
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);
    let config_arc = Arc::new(config);
    
    let registry = Arc::new(AdapterRegistry::new());
    
    // Start network first time
    let result1 = registry.start_network("repeated_test".to_string(), config_arc.clone()).await;
    assert!(result1.is_ok(), "First start should succeed");
    
    // Try to start same network again (this will actually add more adapters)
    // This tests the behavior - it may overwrite or add to the existing network
    let _result2 = registry.start_network("repeated_test".to_string(), config_arc).await;
    // The behavior depends on implementation - it might succeed or fail
    // We just verify it doesn't panic
    
    // Clean up
    registry.stop_network("repeated_test").await.ok();
}

// Note: Tests for actual bind failures (port already in use) are harder to
// test reliably in integration tests without actually spawning listeners.
// Those scenarios are better tested in manual/end-to-end tests.
