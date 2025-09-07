use crate::config::config::{Config, ConfigError};

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[cfg(test)] #[test]
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
        
        [groups.core.middleware]
        incoming = []
        outgoing = []

        [endpoints.basic]
        description = "Basic endpoint"
        type = "basic"
        path_prefix = "/basic"

        [backends.test_backend]
        type = "basic"
        targets = ["target_1"]

        [targets.target_1]
        type = "http"
        url = "http://localhost:8081"`
    "#;

    let result = load_config_from_str(toml);
    assert!(result.is_ok());

    let config = result.unwrap();

    // Additional checks to ensure the config fields were parsed correctly
    assert_eq!(config.proxy.id, "harmony-incoming");
    assert_eq!(config.network["default"].interface, "wg0");
    assert_eq!(config.network["default"].http.bind_address, "1.2.3.4");
    assert_eq!(config.endpoints["basic"].path_prefix, "/basic");
    assert_eq!(config.backends["test_backend"].targets, vec!["target_1"]);
    assert_eq!(config.targets["target_1"].url, "http://localhost:8081");
}
