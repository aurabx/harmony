use harmony::adapters::registry::AdapterRegistry;
use harmony::config::config::Config;
use harmony::config::Cli;
use harmony::config::reload::compute_diff;
use harmony::config::watcher::ConfigWatcher;
use harmony::globals;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

/// Helper to create a test config file
fn create_test_config(dir: &TempDir, port: u16) -> PathBuf {
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

/// Helper to update config file with new port
fn update_config_port(config_path: &PathBuf, new_port: u16) {
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
        new_port
    );

    fs::write(config_path, config_content).expect("Failed to update config");
}

/// Helper to add middleware to config
fn add_middleware_to_config(config_path: &PathBuf, port: u16) {
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

    fs::write(config_path, config_content).expect("Failed to update config");
}

#[tokio::test]
async fn test_config_diff_zero_downtime_changes() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 8080);

    // Load initial config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let old_config = Config::from_args(cli);

    // Add middleware (zero-downtime change)
    add_middleware_to_config(&config_path, 8080);

    // Load new config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let new_config = Config::from_args(cli);

    // Compute diff
    let diff = compute_diff(&old_config, &new_config);

    // Verify zero-downtime change detected
    assert!(diff.has_changes());
    assert!(!diff.requires_adapter_restart());
    assert!(diff.zero_downtime_changes.contains(&"middleware".to_string()));
    assert!(diff.adapter_restarts_required.is_empty());
}

#[tokio::test]
async fn test_config_diff_adapter_restart_required() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 8080);

    // Load initial config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let old_config = Config::from_args(cli);

    // Change port (requires adapter restart)
    update_config_port(&config_path, 8081);

    // Load new config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let new_config = Config::from_args(cli);

    // Compute diff
    let diff = compute_diff(&old_config, &new_config);

    // Verify adapter restart required
    assert!(diff.has_changes());
    assert!(diff.requires_adapter_restart());
    assert!(diff.adapter_restarts_required.contains(&"default".to_string()));
}

#[tokio::test]
async fn test_invalid_config_rejected() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 8080);

    // Load valid config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let valid_config = Config::from_args(cli);

    // Set as global config
    globals::set_config(Arc::new(valid_config.clone()));

    // Write invalid config (empty proxy.id)
    let invalid_content = r#"
[proxy]
id = ""
log_level = "info"

[network.default]
interface = "lo0"
"#;
    fs::write(&config_path, invalid_content).expect("Failed to write invalid config");

    // Try to load - should panic during validation
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let result = std::panic::catch_unwind(|| {
        Config::from_args(cli)
    });

    // Verify load failed
    assert!(result.is_err());

    // Verify old config still in globals
    let current_config = globals::get_config().unwrap();
    assert_eq!(current_config.proxy.id, valid_config.proxy.id);
}

#[tokio::test]
async fn test_adapter_registry_start_stop() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 19080); // Use high port to avoid conflicts

    // Load config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let config = Config::from_args(cli);
    let config = Arc::new(config);

    // Create registry
    let registry = Arc::new(AdapterRegistry::new());

    // Start network
    registry
        .start_network("default".to_string(), config.clone())
        .await
        .expect("Failed to start network");

    // Verify network is running
    let running = registry.get_running_networks().await;
    assert!(running.contains(&"default".to_string()));

    // Give adapters time to fully start
    sleep(Duration::from_millis(500)).await;

    // Stop network
    registry
        .stop_network("default")
        .await
        .expect("Failed to stop network");

    // Verify network stopped
    let running = registry.get_running_networks().await;
    assert!(!running.contains(&"default".to_string()));
}

#[tokio::test]
async fn test_adapter_registry_restart() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 19081);

    // Load initial config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let config = Config::from_args(cli);
    let config = Arc::new(config);

    // Create registry and start network
    let registry = Arc::new(AdapterRegistry::new());
    registry
        .start_network("default".to_string(), config.clone())
        .await
        .expect("Failed to start network");

    sleep(Duration::from_millis(500)).await;

    // Update config with new port
    update_config_port(&config_path, 19082);
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let new_config = Arc::new(Config::from_args(cli));

    // Restart network with new config
    registry
        .restart_network("default".to_string(), new_config)
        .await
        .expect("Failed to restart network");

    sleep(Duration::from_millis(500)).await;

    // Verify network still running
    let running = registry.get_running_networks().await;
    assert!(running.contains(&"default".to_string()));

    // Cleanup
    registry.stop_all().await.expect("Failed to stop all");
}

#[tokio::test]
async fn test_network_add_remove() {
    let temp_dir = TempDir::new().unwrap();
    
    // Initial config with one network
    let config_path = temp_dir.path().join("test-config.toml");
    let initial_content = r#"
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

[network.net1]
interface = "lo0"
enable_wireguard = false

[network.net1.http]
bind_address = "127.0.0.1"
bind_port = 19083

[management]
enabled = true
base_path = "/api"
network = "net1"
"#;
    fs::write(&config_path, initial_content).unwrap();
    fs::create_dir_all(temp_dir.path().join("pipelines")).ok();
    fs::create_dir_all(temp_dir.path().join("transforms")).ok();

    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let old_config = Config::from_args(cli);

    // New config with two networks
    let new_content = r#"
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

[network.net1]
interface = "lo0"
enable_wireguard = false

[network.net1.http]
bind_address = "127.0.0.1"
bind_port = 19083

[network.net2]
interface = "lo0"
enable_wireguard = false

[network.net2.http]
bind_address = "127.0.0.1"
bind_port = 19084

[management]
enabled = true
base_path = "/api"
network = "net1"
"#;
    fs::write(&config_path, new_content).unwrap();

    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let new_config = Config::from_args(cli);

    // Compute diff
    let diff = compute_diff(&old_config, &new_config);

    // Verify network addition detected
    assert!(diff.has_changes());
    assert!(diff.requires_adapter_restart());
    assert!(diff.networks_to_add.contains(&"net2".to_string()));
    assert!(diff.networks_to_remove.is_empty());
}

#[tokio::test]
async fn test_zero_downtime_config_swap() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 8080);

    // Load initial config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let initial_config = Config::from_args(cli);
    
    // Set global config
    globals::set_config(Arc::new(initial_config.clone()));

    // Verify initial config
    let current = globals::get_config().unwrap();
    assert!(current.middleware.is_empty());

    // Add middleware
    add_middleware_to_config(&config_path, 8080);
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let new_config = Config::from_args(cli);

    // Swap config
    globals::set_config(Arc::new(new_config));

    // Verify new config active
    let current = globals::get_config().unwrap();
    assert!(!current.middleware.is_empty());
    assert!(current.middleware.contains_key("test_middleware"));
}

#[tokio::test]
#[ignore] // This test spawns actual file watcher - only run manually
async fn test_file_watcher_detects_changes() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_test_config(&temp_dir, 19085);

    // Load initial config
    let cli = Cli::new(config_path.to_string_lossy().to_string());
    let config = Config::from_args(cli);
    globals::set_config(Arc::new(config));

    // Create registry and start watcher
    let registry = Arc::new(AdapterRegistry::new());
    let watcher = ConfigWatcher::new(
        config_path.to_string_lossy().to_string(),
        registry.clone(),
    );

    // Spawn watcher in background
    let watcher_handle = tokio::spawn(async move {
        watcher.start().await
    });

    // Give watcher time to start
    sleep(Duration::from_millis(500)).await;

    // Modify config
    add_middleware_to_config(&config_path, 19085);

    // Wait for reload (debounce + processing)
    sleep(Duration::from_secs(2)).await;

    // Verify config updated
    let current = globals::get_config().unwrap();
    assert!(current.middleware.contains_key("test_middleware"));

    // Cleanup
    watcher_handle.abort();
    registry.stop_all().await.ok();
}
