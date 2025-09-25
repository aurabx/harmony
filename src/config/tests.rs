use std::collections::HashMap;
use serde_json::Value;
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

#[cfg(test)]
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

        [network.default.http]
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
    // Helper functions to read values from the `options` maps
    // -----------------------------------------------------------------
    fn get_str_option(
        opt: &Option<HashMap<String, Value>>,
        key: &str,
    ) -> String {
        opt.as_ref()
            .and_then(|m| m.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .expect("option missing or not a string")
    }

    fn get_vec_str_option(
        opt: &Option<HashMap<String, Value>>,
        key: &str,
    ) -> Vec<String> {
        opt.as_ref()
            .and_then(|m| m.get(key))
            .and_then(|v| v.as_array())
            .expect("option missing or not an array")
            .iter()
            .map(|v| v.as_str().expect("array element not a string").to_string())
            .collect()
    }

    // -----------------------------------------------------------------
    // Assertions that reflect the data in the TOML above
    // -----------------------------------------------------------------
    // Proxy fields
    assert_eq!(config.proxy.id, "router-test");
    // Network fields
    assert_eq!(config.network["default"].interface, "wg0");
    assert_eq!(
        config.network["default"].http.bind_address,
        "127.0.0.1"
    );
}