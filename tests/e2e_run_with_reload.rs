//! End-to-end tests for hot reload functionality.
//!
//! **Note:** These tests bind to actual network ports and should be run sequentially
//! to avoid port conflicts and timing issues:
//! ```bash
//! cargo test --test e2e_run_with_reload -- --test-threads=1
//! ```

use harmony::config::config::Config;
use harmony::config::Cli;
use reqwest;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::{sleep, timeout};

/// Helper to create a complete test config with pipelines and endpoints
fn create_full_test_config(dir: &TempDir, port: u16) -> PathBuf {
    let config_content = format!(
        r#"
[proxy]
id = "e2e-test-proxy"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "info"
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[runbeam]
enabled = false

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

[pipelines.test_pipeline]
description = "Test pipeline for e2e"
networks = ["default"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
middleware = []

[endpoints.test_endpoint]
service = "http"

[endpoints.test_endpoint.options]
path_prefix = "/test"

[backends.test_backend]
service = "echo"

[services.http]
module = ""

[services.echo]
module = ""
"#,
        port
    );

    let config_path = dir.path().join("e2e-config.toml");
    fs::write(&config_path, config_content).expect("Failed to write test config");

    // Create required directories
    fs::create_dir_all(dir.path().join("pipelines")).ok();
    fs::create_dir_all(dir.path().join("transforms")).ok();

    config_path
}

/// Helper to create config with middleware for hot reload testing
fn create_config_with_middleware(_dir: &TempDir, port: u16) -> String {
    format!(
        r#"
[proxy]
id = "e2e-test-proxy"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "info"
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[runbeam]
enabled = false

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

[pipelines.test_pipeline]
description = "Test pipeline with middleware"
networks = ["default"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
middleware = ["test_middleware"]

[endpoints.test_endpoint]
service = "http"

[endpoints.test_endpoint.options]
path_prefix = "/test"

[backends.test_backend]
service = "echo"

[middleware.test_middleware]
type = "policies"

[[middleware.test_middleware.options.policies]]
id = "allow_policy"
enabled = true

[[middleware.test_middleware.options.policies.rules]]
rule_type = "allow_all"
weight = 100
enabled = true

[middleware_types.policies]
module = ""

[services.http]
module = ""

[services.echo]
module = ""
"#,
        port
    )
}

#[tokio::test]
async fn test_run_with_reload_full_lifecycle() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_full_test_config(&temp_dir, 19200);
    let config_path_str = config_path.to_string_lossy().to_string();

    // Load config and spawn run_with_reload
    let cli = Cli::new(config_path_str.clone());
    let config = Config::from_args(cli);

    let config_path_clone = config_path_str.clone();
    let server_handle = tokio::spawn(async move {
        harmony::run_with_reload(config, Some(config_path_clone)).await;
    });

    // Give server time to start
    sleep(Duration::from_secs(2)).await;

    // Test 1: Verify server is running and responds to requests
    let client = reqwest::Client::new();
    let response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:19200/test/hello").send(),
    )
    .await;

    assert!(
        response.is_ok(),
        "Server should start and respond to requests"
    );
    assert!(
        response.unwrap().is_ok(),
        "HTTP request should succeed after startup"
    );

    // Test 2: Verify management API is accessible
    let mgmt_response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:19200/admin/health").send(),
    )
    .await;

    assert!(
        mgmt_response.is_ok(),
        "Management API should be accessible"
    );

    // Test 3: Hot reload - update config with middleware
    let new_config = create_config_with_middleware(&temp_dir, 19200);
    fs::write(&config_path, new_config).expect("Failed to update config");

    // Wait for file watcher to detect and apply changes (debounce + processing)
    sleep(Duration::from_secs(2)).await;

    // Verify server still responds after config reload
    let post_reload_response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:19200/test/world").send(),
    )
    .await;

    assert!(
        post_reload_response.is_ok(),
        "Server should continue responding after config reload"
    );
    assert_eq!(
        post_reload_response.unwrap().unwrap().status(),
        200,
        "Requests should succeed after hot reload"
    );

    // Test 4: Cleanup
    // Note: abort() doesn't trigger graceful shutdown - it just kills the task.
    // The HTTP servers run in background tasks and will continue running until
    // process exit or until the registry.stop_all() is called (which requires ctrl-c).
    // For now, we just abort the task to clean up the test.
    server_handle.abort();

    println!("✓ Full run_with_reload lifecycle test passed!");
}

#[tokio::test]
async fn test_run_with_reload_multiple_networks() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create config with two networks
    let config_content = r#"
[proxy]
id = "multi-network-test"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "info"
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[runbeam]
enabled = false

[network.net1]
interface = "lo0"
enable_wireguard = false

[network.net1.http]
bind_address = "127.0.0.1"
bind_port = 19201

[network.net2]
interface = "lo0"
enable_wireguard = false

[network.net2.http]
bind_address = "127.0.0.1"
bind_port = 19202

[management]
enabled = true
base_path = "admin"
network = "net1"

[pipelines.pipeline1]
description = "Pipeline on net1"
networks = ["net1"]
endpoints = ["endpoint1"]
backends = ["backend1"]
middleware = []

[pipelines.pipeline2]
description = "Pipeline on net2"
networks = ["net2"]
endpoints = ["endpoint2"]
backends = ["backend2"]
middleware = []

[endpoints.endpoint1]
service = "http"

[endpoints.endpoint1.options]
path_prefix = "/api1"

[endpoints.endpoint2]
service = "http"

[endpoints.endpoint2.options]
path_prefix = "/api2"

[backends.backend1]
service = "echo"

[backends.backend2]
service = "echo"

[services.http]
module = ""

[services.echo]
module = ""
"#;

    let config_path = temp_dir.path().join("multi-net-config.toml");
    fs::write(&config_path, config_content).expect("Failed to write config");
    
    // Create required directories
    fs::create_dir_all(temp_dir.path().join("pipelines")).ok();
    fs::create_dir_all(temp_dir.path().join("transforms")).ok();

    let config_path_str = config_path.to_string_lossy().to_string();

    // Load config and spawn server
    let cli = Cli::new(config_path_str.clone());
    let config = Config::from_args(cli);

    let server_handle = tokio::spawn(async move {
        harmony::run_with_reload(config, Some(config_path_str)).await;
    });

    // Give server time to start both networks
    sleep(Duration::from_secs(2)).await;

    let client = reqwest::Client::new();

    // Test network 1
    let net1_response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:19201/api1/test").send(),
    )
    .await;

    assert!(
        net1_response.is_ok() && net1_response.unwrap().is_ok(),
        "Network 1 should be accessible"
    );

    // Test network 2
    let net2_response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:19202/api2/test").send(),
    )
    .await;

    assert!(
        net2_response.is_ok() && net2_response.unwrap().is_ok(),
        "Network 2 should be accessible"
    );

    // Test management API on network 1
    let mgmt_response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:19201/admin/health").send(),
    )
    .await;

    assert!(
        mgmt_response.is_ok() && mgmt_response.unwrap().is_ok(),
        "Management API should be accessible on net1"
    );

    // Verify network isolation: management API should NOT be on network 2
    let mgmt_net2 = timeout(
        Duration::from_millis(500),
        client.get("http://127.0.0.1:19202/admin/health").send(),
    )
    .await;

    // This should either timeout or return 404
    assert!(
        mgmt_net2.is_err() || mgmt_net2.unwrap().unwrap().status() == 404,
        "Management API should not be accessible on net2"
    );

    // Cleanup
    server_handle.abort();
    sleep(Duration::from_millis(500)).await;

    println!("✓ Multiple networks test passed!");
}

#[tokio::test]
async fn test_run_with_reload_adapter_restart() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_full_test_config(&temp_dir, 19203);
    let config_path_str = config_path.to_string_lossy().to_string();

    // Load config and spawn server
    let cli = Cli::new(config_path_str.clone());
    let config = Config::from_args(cli);

    let config_path_clone = config_path_str.clone();
    let server_handle = tokio::spawn(async move {
        harmony::run_with_reload(config, Some(config_path_clone)).await;
    });

    // Give server time to start
    sleep(Duration::from_secs(2)).await;

    let client = reqwest::Client::new();

    // Verify initial server is running
    let initial_response = client
        .get("http://127.0.0.1:19203/test/initial")
        .send()
        .await;
    assert!(initial_response.is_ok(), "Initial server should respond");

    // Change port (requires adapter restart)
    let new_config = format!(
        r#"
[proxy]
id = "e2e-test-proxy"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "info"
log_to_file = false
log_file_path = ""

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[runbeam]
enabled = false

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 19204

[management]
enabled = true
base_path = "admin"
network = "default"

[pipelines.test_pipeline]
description = "Test pipeline for e2e"
networks = ["default"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
middleware = []

[endpoints.test_endpoint]
service = "http"

[endpoints.test_endpoint.options]
path_prefix = "/test"

[backends.test_backend]
service = "echo"

[services.http]
module = ""

[services.echo]
module = ""
"#
    );

    fs::write(&config_path, new_config).expect("Failed to update config");

    // Wait for file watcher to detect and apply (adapter restart takes longer)
    // Give extra time for the old adapter to fully shut down and release the port
    sleep(Duration::from_secs(4)).await;

    // Old port should no longer respond
    let old_port_response = timeout(
        Duration::from_millis(500),
        client.get("http://127.0.0.1:19203/test/old").send(),
    )
    .await;

    assert!(
        old_port_response.is_err() || old_port_response.unwrap().is_err(),
        "Old port should not respond after adapter restart"
    );

    // New port should respond
    let new_port_response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:19204/test/new").send(),
    )
    .await;

    assert!(
        new_port_response.is_ok() && new_port_response.unwrap().is_ok(),
        "New port should respond after adapter restart"
    );

    // Cleanup
    server_handle.abort();
    sleep(Duration::from_millis(500)).await;

    println!("✓ Adapter restart test passed!");
}

#[tokio::test]
async fn test_run_with_reload_invalid_config_rejected() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = create_full_test_config(&temp_dir, 19205);
    let config_path_str = config_path.to_string_lossy().to_string();

    // Load config and spawn server
    let cli = Cli::new(config_path_str.clone());
    let config = Config::from_args(cli);

    let config_path_clone = config_path_str.clone();
    let server_handle = tokio::spawn(async move {
        harmony::run_with_reload(config, Some(config_path_clone)).await;
    });

    // Give server time to start
    sleep(Duration::from_secs(2)).await;

    let client = reqwest::Client::new();

    // Verify server is running
    let initial_response = client
        .get("http://127.0.0.1:19205/test/before")
        .send()
        .await;
    assert!(initial_response.is_ok(), "Server should be running");

    // Write invalid config (empty proxy.id)
    let invalid_config = r#"
[proxy]
id = ""
log_level = "info"

[network.default]
interface = "lo0"
"#;

    fs::write(&config_path, invalid_config).expect("Failed to write invalid config");

    // Wait for file watcher to attempt reload
    sleep(Duration::from_secs(2)).await;

    // Server should still be running with old config
    let post_invalid_response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:19205/test/after").send(),
    )
    .await;

    assert!(
        post_invalid_response.is_ok() && post_invalid_response.unwrap().is_ok(),
        "Server should continue running with old config after invalid config rejected"
    );

    // Cleanup
    server_handle.abort();
    sleep(Duration::from_millis(500)).await;

    println!("✓ Invalid config rejection test passed!");
}
