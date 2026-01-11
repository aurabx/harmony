use std::env;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_env_substitution_in_config_loading() {
    // Set up environment variables
    env::set_var("TEST_PROXY_ID", "test-proxy");
    env::set_var("TEST_LOG_LEVEL", "info");
    env::set_var("TEST_HOST", "127.0.0.1");
    env::set_var("TEST_PORT", "8080");

    // Create a temporary directory with test config
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("test-config.toml");

    // Create a config file with environment variable references
    let config_content = r#"
[proxy]
id = "$TEST_PROXY_ID"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "$TEST_LOG_LEVEL"
log_to_file = false

[network.test_network]
enable_wireguard = false
interface = "lo"

[network.test_network.http]
bind_address = "$TEST_HOST"
bind_port = $TEST_PORT

[endpoints.test_endpoint]
service = "http"

[backends.test_backend]
service = "http"

[pipelines.test_pipeline]
description = "Test pipeline"
networks = ["test_network"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
"#;

    fs::write(&config_path, config_content).expect("Failed to write config file");

    // Load the config using the harmony configuration system
    let contents = fs::read_to_string(&config_path).expect("Failed to read config");
    
    // Verify that the raw content has environment variable references
    assert!(contents.contains("$TEST_PROXY_ID"));
    assert!(contents.contains("$TEST_LOG_LEVEL"));
    assert!(contents.contains("$TEST_HOST"));
    assert!(contents.contains("$TEST_PORT"));

    // Parse with harmony's config system (which includes env substitution)
    use harmony::config::env_substitution::substitute_env_vars;
    let (substituted, _audit) = substitute_env_vars(&contents);

    // Verify substitutions happened
    assert!(substituted.contains("test-proxy"));
    assert!(substituted.contains("127.0.0.1"));
    assert!(substituted.contains("8080"));
    assert!(!substituted.contains("$TEST_PROXY_ID"));
    assert!(!substituted.contains("$TEST_HOST"));

    // Verify it can be parsed as valid TOML
    let parsed: toml::Table = toml::from_str(&substituted).expect("Failed to parse substituted config");
    
    // Verify values were substituted correctly
    assert_eq!(
        parsed.get("proxy").and_then(|p| p.get("id")).and_then(|v| v.as_str()),
        Some("test-proxy")
    );
    assert_eq!(
        parsed.get("logging").and_then(|l| l.get("log_level")).and_then(|v| v.as_str()),
        Some("info")
    );
    assert_eq!(
        parsed.get("network")
            .and_then(|n| n.get("test_network"))
            .and_then(|tn| tn.get("http"))
            .and_then(|h| h.get("bind_address"))
            .and_then(|v| v.as_str()),
        Some("127.0.0.1")
    );
    assert_eq!(
        parsed.get("network")
            .and_then(|n| n.get("test_network"))
            .and_then(|tn| tn.get("http"))
            .and_then(|h| h.get("bind_port"))
            .and_then(|v| v.as_integer()),
        Some(8080)
    );
}

#[test]
fn test_env_substitution_missing_variables() {
    // Create a temporary directory with test config
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_path = temp_dir.path().join("test-config-missing.toml");

    // Create a config file with a missing environment variable
    let config_content = r#"
[proxy]
id = "$DEFINITELY_NOT_SET_VARIABLE_XYZ"
pipelines_path = "pipelines"
transforms_path = "transforms"

[logging]
log_level = "info"
log_to_file = false

[network.test_network]
enable_wireguard = false
interface = "lo"

[network.test_network.http]
bind_address = "127.0.0.1"
bind_port = 8080

[endpoints.test_endpoint]
service = "http"

[backends.test_backend]
service = "http"

[pipelines.test_pipeline]
description = "Test pipeline"
networks = ["test_network"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
"#;

    fs::write(&config_path, config_content).expect("Failed to write config file");

    // Load and substitute
    let contents = fs::read_to_string(&config_path).expect("Failed to read config");
    use harmony::config::env_substitution::substitute_env_vars;
    let (substituted, _audit) = substitute_env_vars(&contents);

    // Verify the missing variable was replaced with empty string
    assert!(substituted.contains("id = \"\""));
    
    // Verify it can still be parsed as valid TOML
    let parsed: toml::Table = toml::from_str(&substituted).expect("Failed to parse substituted config");
    assert_eq!(
        parsed.get("proxy").and_then(|p| p.get("id")).and_then(|v| v.as_str()),
        Some("")
    );
}

#[test]
fn test_env_substitution_escaped_dollar() {
    let config_content = r#"
[proxy]
id = "test-proxy"
pipelines_path = "pipelines"
transforms_path = "transforms"
description = "Cost is $$100"

[logging]
log_level = "info"
log_to_file = false

[network.test_network]
enable_wireguard = false
interface = "lo"

[network.test_network.http]
bind_address = "127.0.0.1"
bind_port = 8080

[endpoints.test_endpoint]
service = "http"

[backends.test_backend]
service = "http"

[pipelines.test_pipeline]
description = "Test pipeline"
networks = ["test_network"]
endpoints = ["test_endpoint"]
backends = ["test_backend"]
"#;

    use harmony::config::env_substitution::substitute_env_vars;
    let (substituted, _audit) = substitute_env_vars(&config_content);

    // Verify escaped dollar signs are preserved
    assert!(substituted.contains("Cost is $100"));
    assert!(!substituted.contains("$$100"));

    // Verify it can be parsed as valid TOML
    let parsed: toml::Table = toml::from_str(&substituted).expect("Failed to parse substituted config");
    assert_eq!(
        parsed.get("proxy").and_then(|p| p.get("description")).and_then(|v| v.as_str()),
        Some("Cost is $100")
    );
}
