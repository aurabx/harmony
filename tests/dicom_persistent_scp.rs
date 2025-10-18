use harmony::{config::config::Config, collect_required_dimse_scps};
use serde_json::json;

// Helper function to create a test config with DICOM backends
fn create_test_config_with_backends(backends: Vec<(&str, &str, serde_json::Value)>) -> Config {
    let mut config = Config::default();
    
    for (name, service, options) in backends {
        config.backends.insert(
            name.to_string(),
            harmony::models::backends::backends::Backend {
                service: service.to_string(),
                options: Some(options.as_object().unwrap().iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()),
            },
        );
    }
    
    config
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

        let specs = collect_required_dimse_scps(&config);
        
        assert_eq!(specs.len(), 1);
        let spec = &specs[0];
        assert_eq!(spec.backend_name, "test_pacs");
        assert_eq!(spec.port, 11112);
        assert_eq!(spec.local_aet, "TEST_AET");
        assert_eq!(spec.bind_addr, "127.0.0.1".parse::<std::net::IpAddr>().unwrap());
        assert_eq!(spec.storage_dir, std::path::PathBuf::from("./tmp/dimse"));
        assert!(spec.enable_echo);
        assert!(spec.enable_find);
        assert!(spec.enable_move);
    }

    #[test]
    fn test_defaults_applied_correctly() {
        let config = create_test_config_with_backends(vec![(
            "minimal_pacs",
            "dicom",
            json!({
                "persistent_store_scp": true,
                "incoming_store_port": 4242
            }),
        )]);

        let specs = collect_required_dimse_scps(&config);
        
        assert_eq!(specs.len(), 1);
        let spec = &specs[0];
        assert_eq!(spec.backend_name, "minimal_pacs");
        assert_eq!(spec.port, 4242);
        assert_eq!(spec.local_aet, "HARMONY_SCU"); // default
        assert_eq!(spec.bind_addr, "0.0.0.0".parse::<std::net::IpAddr>().unwrap()); // default
        assert_eq!(spec.storage_dir, std::path::PathBuf::from("./tmp/dimse")); // default
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

        let specs = collect_required_dimse_scps(&config);
        
        // Should be empty since none are DICOM backends
        assert_eq!(specs.len(), 0);
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

        let specs = collect_required_dimse_scps(&config);
        
        assert_eq!(specs.len(), 0);
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

        let specs = collect_required_dimse_scps(&config);
        
        assert_eq!(specs.len(), 0);
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

        let specs = collect_required_dimse_scps(&config);
        
        // Should be empty since incoming_store_port is required
        assert_eq!(specs.len(), 0);
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

        let specs = collect_required_dimse_scps(&config);
        
        // Should be empty since ports are out of valid range
        assert_eq!(specs.len(), 0);
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

        let specs = collect_required_dimse_scps(&config);
        
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].storage_dir, std::path::PathBuf::from("/custom/path/dicom"));
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

        let specs = collect_required_dimse_scps(&config);
        
        assert_eq!(specs.len(), 1);
        let spec = &specs[0];
        assert!(!spec.enable_echo);
        assert!(!spec.enable_find);
        assert!(!spec.enable_move);
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

        let specs = collect_required_dimse_scps(&config);
        
        assert_eq!(specs.len(), 2);
        
        let ports: Vec<u16> = specs.iter().map(|s| s.port).collect();
        assert!(ports.contains(&11112));
        assert!(ports.contains(&11113));
        assert!(!ports.contains(&11114));  // Should not include the disabled one
    }

    #[test]
    fn test_no_backends() {
        let config = Config::default();
        let specs = collect_required_dimse_scps(&config);
        assert_eq!(specs.len(), 0);
    }
}