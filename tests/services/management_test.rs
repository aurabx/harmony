use harmony::config::config::Config;
use harmony::models::endpoints::endpoint::Endpoint;
use harmony::models::services::services::ServiceType;
use serde_json::json;
use std::collections::HashMap;
use tokio;

#[tokio::test]
async fn test_management_service_enabled() {
    let mut config = Config::default();
    config.management.enabled = true;
    config.management.base_path = "admin".to_string();

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
    assert_eq!(routes.len(), 2);
    let paths: Vec<_> = routes.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"admin/info"));
    assert!(paths.contains(&"admin/pipelines"));
}

#[tokio::test]
async fn test_management_service_disabled() {
    let mut config = Config::default();
    config.management.enabled = false;

    // Load the configuration - this should not inject the management service
    config.inject_management_service();

    // Verify no endpoint was created
    assert!(!config.endpoints.contains_key("management"));

    // Verify no pipeline was created
    assert!(!config.pipelines.contains_key("management"));
}
