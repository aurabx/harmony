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

        [groups.core]
        description = "Core group"
        networks = ["default"]
        endpoints = []
        backends = []
        peers = []
        
        [groups.core.middleware]
        incoming = []
        outgoing = []

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
        
        [network.default.http]
        bind_address = "1.2.3.4"
        bind_port = 8080

        # Define one group that references the existing network and known middleware
        [groups.core]
        description = "Core group"
        networks = ["default"]
        endpoints = ["ep1"]
        backends = ["b1"]
        
        [groups.core.middleware]
        incoming = ["jwt_auth"]
        outgoing = []

        # Provide middleware configuration for referenced middleware
        [middleware.jwt_auth]
        public_key_path = "/tmp/dummy_pub.pem"

        # Define an endpoint referenced by the group (FHIR doesn't trigger DICOM validation)
        [endpoints.ep1]
        type = "fhir"
        path_prefix = "/fhir"

        # Define a DICOM backend with a valid non-zero port
        [backends.b1]
        type = { type = "dicom", aet = "TEST", host = "127.0.0.1", port = 104 }
    "#;

    let result = load_config_from_str(toml);
    assert!(result.is_ok());
}

#[test]
fn test_missing_group_networks_fails() {
    let toml = r#"
        [proxy]
        id = "jdx-1"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "1.2.3.4"
        bind_port = 8080

        [groups.core]
        description = "Core group"
        networks = []
        endpoints = []
        backends = []
        
        [groups.core.middleware]
        incoming = []
        outgoing = []
    "#;

    // Expect InvalidGroup due to missing networks
    let result = load_config_from_str(toml);
    assert!(matches!(
        result,
        Err(ConfigError::InvalidGroup { name: _, reason: _ })
    ));
}

#[test]
fn test_unknown_network_in_group_fails() {
    let toml = r#"
        [proxy]
        id = "jdx-1"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "1.2.3.4"
        bind_port = 8080

        [groups.core]
        description = "Core group"
        networks = ["ghost_group"]
        endpoints = []
        backends = []
        
        [groups.core.middleware]
        incoming = []
        outgoing = []
    "#;

    let result = load_config_from_str(toml);
    assert!(matches!(
        result,
        Err(ConfigError::UnknownReference { group: _, kind: _, value: _ })
    ));
}
