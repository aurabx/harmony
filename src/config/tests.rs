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
        config.network["default"]
            .tcp_config
            .as_ref()
            .expect("tcp_config should be present")
            .bind_address,
        "127.0.0.1"
    );
}

#[test]
fn test_peers_and_targets_config() {
    let toml = r#"
        [proxy]
        id = "test-proxy"
        
        [authentications.none-auth]
        id = "none-auth"
        method = "none"
        
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
        authentication = "none-auth"
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_ok(),
        "Configuration with peers and targets should parse"
    );

    let config = result.unwrap();

    // Check Peer
    let peer = config
        .peers
        .get("hospital_a")
        .expect("Peer hospital_a not found");
    assert_eq!(peer.name, Some("Hospital A".to_string()));
    assert_eq!(peer.protocol, Some("dicom".to_string()));
    assert_eq!(peer.connection.host, "10.0.0.1");
    assert_eq!(peer.connection.port, Some(11112));

    // Check Target
    let target = config
        .targets
        .get("pacs_archive")
        .expect("Target pacs_archive not found");
    assert_eq!(target.name, Some("Main PACS".to_string()));
    assert_eq!(target.protocol, Some("dicom".to_string()));
    assert_eq!(target.connection.host, "pacs.internal");
    assert_eq!(target.connection.port, Some(104));
    assert!(target.authentication.is_some());
    assert_eq!(target.authentication.as_ref().unwrap(), "none-auth");
}

/// Minimal valid config with one pipeline using middleware instance `gate`.
/// `middleware_toml` supplies the `[middleware.gate]` block and any
/// top-level policy/rule definitions it needs.
fn config_with_gate_middleware(middleware_toml: &str) -> String {
    format!(
        r#"
        [proxy]
        id = "middleware-validation-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = ["basic"]
        backends = []
        middleware = ["gate"]

        [endpoints.basic]
        service = "http"
        [endpoints.basic.options]
        path_prefix = "/basic"

        [services.http]
        module = ""

        [middleware_types.policies]
        module = ""

        {middleware_toml}
    "#
    )
}

#[test]
fn test_policies_middleware_rejects_inline_policy_tables() {
    // Pre-v1.7 inline format: policies were tables, not string IDs. The
    // middleware cannot be built from this, so validation must reject it
    // instead of letting every request fail with HTTP 500.
    let toml = config_with_gate_middleware(
        r#"
        [middleware.gate]
        type = "policies"

        [[middleware.gate.options.policies]]
        id = "allow_policy"
        enabled = true

        [[middleware.gate.options.policies.rules]]
        rule_type = "allow_all"
        weight = 100
        enabled = true
    "#,
    );

    match load_config_from_str(&toml) {
        Err(ConfigError::InvalidMiddleware { name, reason }) => {
            assert_eq!(name, "gate");
            assert!(
                reason.contains("must be string IDs"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!("expected InvalidMiddleware, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn test_policies_middleware_accepts_policy_id_references() {
    let toml = config_with_gate_middleware(
        r#"
        [middleware.gate]
        type = "policies"
        [middleware.gate.options]
        policies = ["allow_policy"]

        [policies.allow_policy]
        id = "allow_policy"
        rules = ["allow_everyone"]

        [rules.allow_everyone]
        id = "allow_everyone"
        type = "allow_all"
    "#,
    );

    let result = load_config_from_str(&toml);
    assert!(
        result.is_ok(),
        "expected valid config, got {:?}",
        result.err()
    );
}

#[test]
fn test_policies_middleware_rejects_unknown_policy_id() {
    let toml = config_with_gate_middleware(
        r#"
        [middleware.gate]
        type = "policies"
        [middleware.gate.options]
        policies = ["missing_policy"]
    "#,
    );

    match load_config_from_str(&toml) {
        Err(ConfigError::InvalidMiddleware { name, reason }) => {
            assert_eq!(name, "gate");
            assert!(
                reason.contains("missing_policy"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!("expected InvalidMiddleware, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn test_transform_middleware_rejects_missing_spec_file() {
    let toml = config_with_gate_middleware(
        r#"
        [middleware.gate]
        type = "transform"
        [middleware.gate.options]
        spec_path = "/nonexistent/harmony-validation-test/spec.json"
    "#,
    );

    match load_config_from_str(&toml) {
        Err(ConfigError::InvalidMiddleware { name, reason }) => {
            assert_eq!(name, "gate");
            assert!(
                reason.contains("Failed to read JOLT spec file"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!("expected InvalidMiddleware, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn test_jwt_auth_middleware_rejects_missing_key_without_panicking() {
    let toml = config_with_gate_middleware(
        r#"
        [middleware.gate]
        type = "jwt_auth"
        [middleware.gate.options]
        public_key_path = "/nonexistent/harmony-validation-test/key.pem"
    "#,
    );

    match load_config_from_str(&toml) {
        Err(ConfigError::InvalidMiddleware { name, reason }) => {
            assert_eq!(name, "gate");
            assert!(
                reason.contains("failed to read RSA public key"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!("expected InvalidMiddleware, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn test_jwt_auth_hs256_without_secret_is_rejected() {
    let toml = config_with_gate_middleware(
        r#"
        [middleware.gate]
        type = "jwt_auth"
        [middleware.gate.options]
        use_hs256 = true
    "#,
    );
    let config: Config = toml::from_str(&toml).expect("TOML parse error");

    match config.validate() {
        Err(ConfigError::InvalidMiddleware { name, reason }) => {
            assert_eq!(name, "gate");
            assert!(
                reason.contains("requires a non-empty hs256_secret"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!("expected InvalidMiddleware, got {:?}", other),
    }
}

#[test]
fn test_bare_policies_reference_in_pipeline_is_rejected() {
    // No [middleware.policies] block: the pipeline name is built as a type
    // with empty options, which the policies middleware cannot accept.
    let toml = config_with_gate_middleware("")
        .replace(r#"middleware = ["gate"]"#, r#"middleware = ["policies"]"#);

    match load_config_from_str(&toml) {
        Err(ConfigError::InvalidMiddleware { name, reason }) => {
            assert_eq!(name, "policies");
            assert!(
                reason.contains("'policies' array"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!("expected InvalidMiddleware, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn test_bare_json_extractor_reference_in_pipeline_is_accepted() {
    let toml = config_with_gate_middleware("").replace(
        r#"middleware = ["gate"]"#,
        r#"middleware = ["json_extractor"]"#,
    );

    let result = load_config_from_str(&toml);
    assert!(
        result.is_ok(),
        "expected valid config, got {:?}",
        result.err()
    );
}

#[test]
fn test_dimse_to_jmix_middleware_rejects_empty_upstream_path() {
    let toml = config_with_gate_middleware(
        r#"
        [middleware.gate]
        type = "dimse_to_jmix"
        [middleware.gate.options]
        upstream_path = " "
    "#,
    );

    match load_config_from_str(&toml) {
        Err(ConfigError::InvalidMiddleware { name, reason }) => {
            assert_eq!(name, "gate");
            assert!(
                reason.contains("upstream_path must not be empty"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!("expected InvalidMiddleware, got {:?}", other.map(|_| ())),
    }
}
