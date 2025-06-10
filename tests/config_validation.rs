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
        id = "harmony-incoming"
        log_level = "info"
        store_dir = "/var/lib/jmix/studies"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "1.2.3.4"
        bind_port = 8080

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

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        peers = []
        
        [network.default.http]
        bind_address = "1.2.3.4"
        bind_port = 8080

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

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        peers = []
        
        [network.default.http]
        bind_address = "1.2.3.4"
        bind_port = 8080

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

    // Match both fields of `InvalidGroup`, but use `_` to ignore field contents
    let result = load_config_from_str(toml);
    assert!(matches!(
        result,
        Err(ConfigError::InvalidGroup { name: _, reason: _ })
    ));
}

#[test]
fn test_unknown_group_in_transform_fails() {
    let toml = r#"
        [proxy]
        id = "jdx-1"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        peers = []
        
        [network.default.http]
        bind_address = "1.2.3.4"
        bind_port = 8080

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
    assert!(matches!(
        result,
        Err(ConfigError::UnknownReference { group: _, kind: _, value: _ })
    ));
}
