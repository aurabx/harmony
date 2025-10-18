use harmony::config::config::Config;
use std::collections::HashMap;
use tokio;

#[tokio::test]
async fn test_management_service_enabled() {
    let mut config = Config::default();

    // Add default network configuration
    let mut network_config = harmony::models::network::config::NetworkConfig::default();
    network_config.interface = "default".to_string();
    network_config.http.bind_address = "127.0.0.1".to_string();
    network_config.http.bind_port = 8080;
    config.network.insert("default".to_string(), network_config);

    config.management.enabled = true;
    config.management.base_path = "admin".to_string();
    config.management.network = Some("default".to_string());

    // Load the configuration - this should inject the management service
    config.inject_management_service();

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
    assert_eq!(routes.len(), 3); // Updated to match actual count
    assert!(paths.contains(&"/admin/info"));
    assert!(paths.contains(&"/admin/pipelines"));
    assert!(paths.contains(&"/admin/routes"));
}

#[tokio::test]
async fn test_management_service_disabled() {
    let mut config = Config::default();

    // Add default network configuration
    let mut network_config = harmony::models::network::config::NetworkConfig::default();
    network_config.interface = "default".to_string();
    network_config.http.bind_address = "127.0.0.1".to_string();
    network_config.http.bind_port = 8080;
    config.network.insert("default".to_string(), network_config);

    config.management.enabled = false;

    // Load the configuration - this should not inject the management service
    config.inject_management_service();

    // Verify no endpoint was created
    assert!(!config.endpoints.contains_key("management"));

    // Verify no pipeline was created
    assert!(!config.pipelines.contains_key("management"));
}
