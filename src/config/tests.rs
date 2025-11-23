#![cfg(test)]

use toml; // bring the toml crate into scope

use crate::config::config::{Config, ConfigError};

/// Parse a TOML string into a `Config` and run the project's validation logic.
fn load_config_from_str(toml_str: &str) -> Result<Config, ConfigError> {
    // `toml::from_str` deserialises the string according to the `Config` struct.
    let cfg: Config = toml::from_str(toml_str).expect("TOML parse error");
    // Validate cross‑references, required fields, etc.
    cfg.validate()?;
    Ok(cfg)
}

#[test]
fn test_basic_config() {
    // This TOML matches the current configuration schema.
    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.tcp_config]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = ["basic", "fhir"]
        backends = []
        middleware = []

        [endpoints.basic]
        service = "http"
        [endpoints.basic.options]
        path_prefix = "/basic"

        [endpoints.fhir]
        service = "fhir"
        [endpoints.fhir.options]
        path_prefix = "/fhir"

        [services.http]
        module = ""

        [services.fhir]
        module = ""
    "#;

    // -----------------------------------------------------------------
    // Load & validate the configuration
    // -----------------------------------------------------------------
    let result = load_config_from_str(toml);
    assert!(result.is_ok(), "Configuration should parse and validate");

    let config = result.unwrap();

    // -----------------------------------------------------------------
    // Assertions that reflect the data in the TOML above
    // -----------------------------------------------------------------
    // Proxy fields
    assert_eq!(config.proxy.id, "router-test");
    // Network fields
    assert_eq!(config.network["default"].interface, "wg0");
    assert_eq!(
        config.network["default"].tcp_config.bind_address,
        "127.0.0.1"
    );
}

#[test]
fn test_peers_and_targets_config() {
    let toml = r#"
        [proxy]
        id = "test-proxy"
        
        [peers.hospital_a]
        name = "Hospital A"
        type = "dicom"
        connection.host = "10.0.0.1"
        connection.port = 11112
        
        [targets.pacs_archive]
        name = "Main PACS"
        type = "dicom"
        connection.host = "pacs.internal"
        connection.port = 104
        authentication.method = "none"
    "#;

    let result = load_config_from_str(toml);
    assert!(result.is_ok(), "Configuration with peers and targets should parse");
    
    let config = result.unwrap();
    
    // Check Peer
    let peer = config.peers.get("hospital_a").expect("Peer hospital_a not found");
    assert_eq!(peer.name, Some("Hospital A".to_string()));
    assert_eq!(peer.r#type, "dicom");
    assert_eq!(peer.connection.host, "10.0.0.1");
    assert_eq!(peer.connection.port, Some(11112));
    
    // Check Target
    let target = config.targets.get("pacs_archive").expect("Target pacs_archive not found");
    assert_eq!(target.name, Some("Main PACS".to_string()));
    assert_eq!(target.r#type, "dicom");
    assert_eq!(target.connection.host, "pacs.internal");
    assert_eq!(target.connection.port, Some(104));
    assert!(target.authentication.is_some());
    assert_eq!(target.authentication.as_ref().unwrap().method, "none");
}
