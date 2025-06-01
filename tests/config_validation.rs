use harmony::config::{Config, ConfigError};

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[test]
fn test_basic_config() {
    let toml = r#"
        [proxy]
        id = "jdx-1"
        log_level = "info"
        store_dir = "/tmp"

        [network]
        enable_wireguard = false
        interface = "wg0"
    "#;

    let result = load_config_from_str(toml);
    assert!(result.is_ok());
}

#[test]
fn test_valid_config_passes() {
    let toml = r#"
        [proxy]
        id = "jdx-1"
        log_level = "info"
        store_dir = "/tmp"

        [network]
        enable_wireguard = false
        interface = "wg0"
        peers = []

        [endpoints.ep1]
        type = "fhir"
        path_prefix = "/fhir"
        group = "external_fhir"
        middleware = ["test"]

        [internal_services.svc1]
        type = "dicom"
        aet = "TEST"
        host = "127.0.0.1"
        port = 104
        group = "internal_dicom"
        middleware = ["test"]

        [transform_rules.rule1]
        from_group = "external_fhir"
        to_group = "internal_dicom"
        transform_chain = ["fhir_to_dicom"]
    "#;

    let result = load_config_from_str(toml);
    assert!(result.is_ok());
}

#[test]
fn test_missing_endpoint_group_fails() {
    let toml = r#"
        [proxy]
        id = "jdx-1"
        log_level = "info"
        store_dir = "/tmp"

        [network]
        enable_wireguard = false
        interface = "wg0"
        peers = []

        [endpoints.ep1]
        type = "fhir"
        path_prefix = "/fhir"
        group = ""

        [internal_services.svc1]
        type = "dicom"
        aet = "TEST"
        host = "127.0.0.1"
        port = 104
        group = "internal_dicom"

        [transform_rules.rule1]
        from_group = "external_fhir"
        to_group = "internal_dicom"
        transform_chain = ["fhir_to_dicom"]
    "#;

    let result = load_config_from_str(toml);
    assert!(matches!(result, Err(ConfigError::InvalidEndpointGroup(_))));
}

#[test]
fn test_unknown_group_in_transform_fails() {
    let toml = r#"
        [proxy]
        id = "jdx-1"
        log_level = "info"
        store_dir = "/tmp"

        [network]
        enable_wireguard = false
        interface = "wg0"
        peers = []

        [endpoints.ep1]
        type = "fhir"
        path_prefix = "/fhir"
        group = "external_fhir"

        [internal_services.svc1]
        type = "dicom"
        aet = "TEST"
        host = "127.0.0.1"
        port = 104
        group = "internal_dicom"

        [transform_rules.rule1]
        from_group = "external_fhir"
        to_group = "ghost_group"
        transform_chain = ["fhir_to_dicom"]
    "#;

    let result = load_config_from_str(toml);
    assert!(matches!(result, Err(ConfigError::UnknownGroup { .. })));
}
