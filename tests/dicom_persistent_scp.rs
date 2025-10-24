use harmony::config::config::Config;
use harmony::models::backends::backends::Backend;
use harmony::models::endpoints::endpoint::Endpoint;
use harmony::models::network::config::{NetworkConfig, HttpConfig};
use harmony::models::pipelines::config::Pipeline;
use serde_json::json;
use std::collections::HashMap;

// Helper function to create a test config with DICOM backends in a pipeline
fn create_test_config_with_backends(
    backends: Vec<(&str, &str, serde_json::Value)>,
) -> Config {
    let mut config = Config::default();
    
    // Create a test network
    let mut network = NetworkConfig::default();
    network.http = HttpConfig {
        bind_address: "127.0.0.1".to_string(),
        bind_port: 8080,
    };
    config.network.insert("test_network".to_string(), network);
    
    let mut backend_names = Vec::new();
    
    // Add backends
    for (name, service, options) in backends {
        config.backends.insert(
            name.to_string(),
            Backend {
                service: service.to_string(),
                options: Some(
                    options
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
            },
        );
        backend_names.push(name.to_string());
    }
    
    // Create a pipeline that uses these backends
    if !backend_names.is_empty() {
        config.pipelines.insert(
            "test_pipeline".to_string(),
            Pipeline {
                description: "Test pipeline for DICOM backends".to_string(),
                networks: vec!["test_network".to_string()],
                endpoints: vec![],
                backends: backend_names,
                middleware: vec![],
            },
        );
    }
    
    config
}

// Helper function to create test config with DIMSE/dicom_scp endpoints
fn create_test_config_with_endpoints(
    endpoints: Vec<(&str, &str, serde_json::Value)>,
) -> Config {
    let mut config = Config::default();
    
    // Create a test network
    let mut network = NetworkConfig::default();
    network.http = HttpConfig {
        bind_address: "127.0.0.1".to_string(),
        bind_port: 8080,
    };
    config.network.insert("test_network".to_string(), network);
    
    let mut endpoint_names = Vec::new();
    
    // Add endpoints
    for (name, service, options) in endpoints {
        config.endpoints.insert(
            name.to_string(),
            Endpoint {
                service: service.to_string(),
                options: Some(
                    options
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
            },
        );
        endpoint_names.push(name.to_string());
    }
    
    // Create a pipeline that uses these endpoints
    if !endpoint_names.is_empty() {
        config.pipelines.insert(
            "test_pipeline".to_string(),
            Pipeline {
                description: "Test pipeline for DIMSE endpoints".to_string(),
                networks: vec!["test_network".to_string()],
                endpoints: endpoint_names,
                backends: vec![],
                middleware: vec![],
            },
        );
    }
    
    config
}

// Helper to count how many persistent DICOM SCPs should be started
fn count_expected_persistent_scps(config: &Config, network_name: &str) -> usize {
    let mut count = 0;
    
    for (_, pipeline_cfg) in &config.pipelines {
        if !pipeline_cfg.networks.contains(&network_name.to_string()) {
            continue;
        }
        
        // Check endpoints for dimse/dicom_scp services
        for endpoint_name in &pipeline_cfg.endpoints {
            if let Some(endpoint) = config.endpoints.get(endpoint_name) {
                if matches!(endpoint.service.as_str(), "dimse" | "dicom_scp") {
                    count += 1;
                }
            }
        }
        
        // Check backends for persistent SCPs
        for backend_name in &pipeline_cfg.backends {
            if let Some(backend) = config.backends.get(backend_name) {
                if backend.service == "dicom" {
                    if let Some(options) = &backend.options {
                        let persistent = options
                            .get("persistent_store_scp")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let has_ports = options.contains_key("host") 
                            && options.contains_key("port");
                        let has_incoming = options.contains_key("incoming_store_port");
                        
                        if (persistent || has_ports) && has_incoming {
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_dicom_backend_with_persistent_scp() {
        let config = create_test_config_with_backends(vec![(
            "test_pacs",
            "dicom",
            json!({
                "persistent_store_scp": true,
                "incoming_store_port": 11112,
                "local_aet": "TEST_AET",
                "bind_addr": "127.0.0.1"
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 1, "Should detect 1 persistent DICOM SCP");
    }

    #[test]
    fn test_minimal_config_detected() {
        let config = create_test_config_with_backends(vec![(
            "minimal_pacs",
            "dicom",
            json!({
                "persistent_store_scp": true,
                "incoming_store_port": 4242
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 1, "Should detect 1 persistent SCP with minimal config");
    }

    #[test]
    fn test_non_dicom_backends_ignored() {
        let config = create_test_config_with_backends(vec![
            (
                "http_backend",
                "http",
                json!({
                    "persistent_store_scp": true,
                    "incoming_store_port": 8080
                }),
            ),
            (
                "fhir_backend", 
                "fhir",
                json!({
                    "persistent_store_scp": true,
                    "incoming_store_port": 9090
                }),
            ),
        ]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 0, "Non-DICOM backends should be ignored");
    }

    #[test]
    fn test_persistent_scp_false_ignored() {
        let config = create_test_config_with_backends(vec![(
            "no_persistent_pacs",
            "dicom",
            json!({
                "persistent_store_scp": false,
                "incoming_store_port": 11112
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 0, "Backends with persistent_store_scp=false should be ignored");
    }

    #[test]
    fn test_missing_persistent_scp_ignored() {
        let config = create_test_config_with_backends(vec![(
            "implicit_false_pacs",
            "dicom",
            json!({
                "incoming_store_port": 11112
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 0, "Backends without persistent_store_scp flag should be ignored");
    }

    #[test] 
    fn test_missing_incoming_store_port_ignored() {
        let config = create_test_config_with_backends(vec![(
            "missing_port_pacs",
            "dicom",
            json!({
                "persistent_store_scp": true
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 0, "Backends without incoming_store_port should be ignored");
    }

    #[test]
    fn test_invalid_incoming_store_port_ignored() {
        let config = create_test_config_with_backends(vec![
            (
                "invalid_port_zero", 
                "dicom",
                json!({
                    "persistent_store_scp": true,
                    "incoming_store_port": 0
                }),
            ),
            (
                "invalid_port_high",
                "dicom", 
                json!({
                    "persistent_store_scp": true,
                    "incoming_store_port": 99999
                }),
            ),
        ]);

        let count = count_expected_persistent_scps(&config, "test_network");
        // Note: Port validation happens at runtime, not during discovery
        // These will be discovered but fail to start
        assert_eq!(count, 2, "Invalid ports are discovered but will fail at startup");
    }

    #[test]
    fn test_custom_storage_dir() {
        let config = create_test_config_with_backends(vec![(
            "custom_storage_pacs",
            "dicom", 
            json!({
                "persistent_store_scp": true,
                "incoming_store_port": 11112,
                "storage_dir": "/custom/path/dicom"
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 1, "Should detect SCP with custom storage dir");
    }

    #[test]
    fn test_feature_flags() {
        let config = create_test_config_with_backends(vec![(
            "feature_flags_pacs",
            "dicom",
            json!({
                "persistent_store_scp": true,
                "incoming_store_port": 11112,
                "enable_echo": false,
                "enable_find": false,
                "enable_move": false
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 1, "Should detect SCP even with disabled feature flags");
    }

    #[test]
    fn test_multiple_dicom_backends() {
        let config = create_test_config_with_backends(vec![
            (
                "pacs1", 
                "dicom",
                json!({
                    "persistent_store_scp": true,
                    "incoming_store_port": 11112
                }),
            ),
            (
                "pacs2",
                "dicom",
                json!({
                    "persistent_store_scp": true, 
                    "incoming_store_port": 11113
                }),
            ),
            (
                "pacs3",
                "dicom",
                json!({
                    "persistent_store_scp": false,
                    "incoming_store_port": 11114  // Should be ignored
                }),
            ),
        ]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 2, "Should detect 2 persistent SCPs and ignore the disabled one");
    }

    #[test]
    fn test_no_backends() {
        let config = Config::default();
        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 0, "Empty config should have no SCPs");
    }
    
    #[test]
    fn test_dicom_scp_endpoint() {
        let config = create_test_config_with_endpoints(vec![(
            "dicom_listener",
            "dicom_scp",
            json!({
                "port": 11112,
                "local_aet": "HARMONY_SCP",
                "bind_addr": "0.0.0.0"
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 1, "Should detect dicom_scp endpoint");
    }
    
    #[test]
    fn test_dimse_endpoint_legacy() {
        let config = create_test_config_with_endpoints(vec![(
            "dimse_listener",
            "dimse",
            json!({
                "port": 11112,
                "local_aet": "HARMONY_SCP"
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 1, "Should detect legacy dimse endpoint");
    }
    
    #[test]
    fn test_backend_with_host_port_detected() {
        // Test the legacy detection: backends with host+port but no persistent_store_scp flag
        let config = create_test_config_with_backends(vec![(
            "legacy_pacs",
            "dicom",
            json!({
                "host": "pacs.example.com",
                "port": 104,
                "incoming_store_port": 11112
            }),
        )]);

        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 1, "Backend with host+port should be detected as persistent SCP");
    }
    
    #[test]
    fn test_mixed_endpoints_and_backends() {
        let mut config = create_test_config_with_endpoints(vec![(
            "scp_endpoint",
            "dicom_scp",
            json!({ "port": 11112 }),
        )]);
        
        // Add a backend to the same pipeline
        config.backends.insert(
            "pacs_backend".to_string(),
            Backend {
                service: "dicom".to_string(),
                options: Some({
                    let mut map = HashMap::new();
                    map.insert(
                        "persistent_store_scp".to_string(),
                        json!(true),
                    );
                    map.insert(
                        "incoming_store_port".to_string(),
                        json!(11113),
                    );
                    map
                }),
            },
        );
        config.pipelines.get_mut("test_pipeline").unwrap().backends.push("pacs_backend".to_string());
        
        let count = count_expected_persistent_scps(&config, "test_network");
        assert_eq!(count, 2, "Should detect both endpoint and backend SCPs");
    }
}
