use harmony::config::config::Config;
use std::collections::HashMap;
use tokio;

#[tokio::test]
async fn test_management_service_enabled() {
    let mut config = Config::default();

    // Add default network configuration
    let mut network_config = harmony::models::network::config::NetworkConfig::default();
    network_config.interface = "default".to_string();
    network_config.tcp_config.bind_address = "127.0.0.1".to_string();
    network_config.tcp_config.bind_port = 8080;
    config.network.insert("default".to_string(), network_config);

    config.management.enabled = true;
    config.management.base_path = "admin".to_string();
    config.management.network = Some("default".to_string());

    // Load the configuration - this should inject the management service
    config.inject_management_service()
        .expect("Failed to inject management service");

    // Verify endpoint was created
    let endpoint = config
        .endpoints
        .get("management")
        .expect("Management endpoint not created");
    assert_eq!(endpoint.service, "management");

    // Verify pipeline was created
    let pipeline = config
        .pipelines
        .get("management")
        .expect("Management pipeline not created");
    assert_eq!(pipeline.endpoints, vec!["management"]);
    assert!(pipeline.middleware.is_empty());

    // Verify service is properly registered
    let service = config
        .services
        .get("management")
        .expect("Management service not registered");
    assert_eq!(service.module, "");

    // Test that the management endpoint can be resolved
    let empty = HashMap::new();
    let endpoint_options = endpoint.options.as_ref().unwrap_or(&empty);
    let service = endpoint
        .resolve_service()
        .expect("Failed to resolve management service");
    service
        .validate(endpoint_options)
        .expect("Service validation failed");

    // Test router configuration
    let routes = service.build_router(endpoint_options);
    let paths: Vec<_> = routes.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(routes.len(), 7); // Updated to match actual count (info, pipelines, routes, authorize, config/status, token, update)
    assert!(paths.contains(&"/admin/info"));
    assert!(paths.contains(&"/admin/pipelines"));
    assert!(paths.contains(&"/admin/routes"));
    assert!(paths.contains(&"/admin/authorize"));
    assert!(paths.contains(&"/admin/config/status"));
    assert!(paths.contains(&"/admin/token"));
    assert!(paths.contains(&"/admin/update"));
}

#[tokio::test]
async fn test_management_service_disabled() {
    let mut config = Config::default();

    // Add default network configuration
    let mut network_config = harmony::models::network::config::NetworkConfig::default();
    network_config.interface = "default".to_string();
    network_config.tcp_config.bind_address = "127.0.0.1".to_string();
    network_config.tcp_config.bind_port = 8080;
    config.network.insert("default".to_string(), network_config);

    config.management.enabled = false;

    // Load the configuration - this should not inject the management service
    config.inject_management_service()
        .expect("Failed to inject management service");

    // Verify no endpoint was created
    assert!(!config.endpoints.contains_key("management"));

    // Verify no pipeline was created
    assert!(!config.pipelines.contains_key("management"));
}

#[tokio::test]
async fn test_management_service_auto_generate_network() {
    let mut config = Config::default();

    // Enable management without specifying a network
    config.management.enabled = true;
    config.management.base_path = "admin".to_string();
    config.management.network = None; // No network specified

    // Inject management service - this should auto-generate a default network
    config.inject_management_service()
        .expect("Failed to inject management service");

    // Verify management network was auto-generated
    assert!(config.network.contains_key("management"));
    let management_network = config.network.get("management").unwrap();
    assert_eq!(management_network.tcp_config.bind_address, "127.0.0.1");
    assert_eq!(management_network.tcp_config.bind_port, 9090);
    assert!(!management_network.enable_wireguard);

    // Verify management.network reference was set
    assert_eq!(config.management.network, Some("management".to_string()));

    // Verify endpoint was created
    assert!(config.endpoints.contains_key("management"));

    // Verify pipeline was created with the auto-generated network
    let pipeline = config.pipelines.get("management").unwrap();
    assert_eq!(pipeline.networks, vec!["management"]);
}

#[tokio::test]
async fn test_management_service_invalid_network_reference() {
    let mut config = Config::default();

    // Add a different network
    let mut network_config = harmony::models::network::config::NetworkConfig::default();
    network_config.interface = "default".to_string();
    network_config.tcp_config.bind_address = "127.0.0.1".to_string();
    network_config.tcp_config.bind_port = 8080;
    config.network.insert("default".to_string(), network_config);

    // Enable management with an invalid network reference
    config.management.enabled = true;
    config.management.base_path = "admin".to_string();
    config.management.network = Some("nonexistent".to_string());

    // Inject management service - this should fail with a clear error
    let result = config.inject_management_service();
    assert!(result.is_err());

    if let Err(err) = result {
        // Verify error message includes available networks
        let error_msg = format!("{:?}", err);
        assert!(error_msg.contains("nonexistent"));
        assert!(error_msg.contains("Available networks"));
        assert!(error_msg.contains("default"));
    }
}
