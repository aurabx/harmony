use harmony::config::config::{Config, ConfigError};

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[test]
fn backend_authentication_resolved_from_target() {
    let toml = r#"
        [proxy]
        id = "backend-auth-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        # Global authentication
        [authentications.api-auth]
        id = "api-auth"
        method = "bearer"
        [authentications.api-auth.options]
        token = "test-token-12345"

        # Target with authentication
        [targets.external-api]
        connection.host = "api.example.com"
        connection.port = 443
        connection.protocol = "https"
        authentication = "api-auth"
        timeout_secs = 30
        max_retries = 3

        [pipelines.api-proxy]
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["api_backend"]
        middleware = []

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/api"

        # Backend references target
        [backends.api_backend]
        service = "http"
        target_ref = "external-api"

        [services.http]
        module = ""
    "#;

    let mut config = load_config_from_str(toml).expect("valid config");
    
    // Resolve references (this should merge target auth to backend)
    harmony::config::resolution::resolve_references(&mut config).expect("resolution should succeed");

    // Verify backend has authentication reference from target
    let backend = config.backends.get("api_backend").expect("backend exists");
    assert_eq!(backend.authentication, Some("api-auth".to_string()));

    // Verify authentication_def was injected into backend options
    if let Some(options) = &backend.options {
        assert!(options.contains_key("authentication_def"), "authentication_def should be in backend options");
        
        // Verify the authentication definition was resolved correctly
        if let Some(auth_def_json) = options.get("authentication_def") {
            let auth_def: harmony::models::connection::AuthenticationDefinition = 
                serde_json::from_value(auth_def_json.clone()).expect("should deserialize");
            
            assert_eq!(auth_def.id, "api-auth");
            assert_eq!(auth_def.method, "bearer");
            assert_eq!(
                auth_def.options.get("token").and_then(|v| v.as_str()),
                Some("test-token-12345")
            );
        }
    }
}

#[test]
fn backend_with_direct_authentication() {
    let toml = r#"
        [proxy]
        id = "backend-auth-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        # Global authentication
        [authentications.basic-auth]
        id = "basic-auth"
        method = "basic"
        [authentications.basic-auth.options]
        username = "admin"
        password = "secret"

        [pipelines.api-proxy]
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["api_backend"]
        middleware = []

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/api"

        # Backend with direct authentication (no target)
        [backends.api_backend]
        service = "http"
        authentication = "basic-auth"
        [backends.api_backend.options]
        base_url = "https://api.example.com"

        [services.http]
        module = ""
    "#;

    let mut config = load_config_from_str(toml).expect("valid config");
    
    harmony::config::resolution::resolve_references(&mut config).expect("resolution should succeed");

    // Verify backend has authentication reference
    let backend = config.backends.get("api_backend").expect("backend exists");
    assert_eq!(backend.authentication, Some("basic-auth".to_string()));

    // Verify authentication_def was injected
    if let Some(options) = &backend.options {
        assert!(options.contains_key("authentication_def"));
        
        if let Some(auth_def_json) = options.get("authentication_def") {
            let auth_def: harmony::models::connection::AuthenticationDefinition = 
                serde_json::from_value(auth_def_json.clone()).expect("should deserialize");
            
            assert_eq!(auth_def.id, "basic-auth");
            assert_eq!(auth_def.method, "basic");
            assert_eq!(
                auth_def.options.get("username").and_then(|v| v.as_str()),
                Some("admin")
            );
            assert_eq!(
                auth_def.options.get("password").and_then(|v| v.as_str()),
                Some("secret")
            );
        }
    }
}

#[test]
fn backend_overrides_target_authentication() {
    let toml = r#"
        [proxy]
        id = "backend-auth-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        # Global authentications
        [authentications.target-auth]
        id = "target-auth"
        method = "bearer"
        [authentications.target-auth.options]
        token = "target-token"

        [authentications.backend-auth]
        id = "backend-auth"
        method = "bearer"
        [authentications.backend-auth.options]
        token = "backend-token"

        # Target with authentication
        [targets.external-api]
        connection.host = "api.example.com"
        connection.port = 443
        connection.protocol = "https"
        authentication = "target-auth"
        timeout_secs = 30
        max_retries = 3

        [pipelines.api-proxy]
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["api_backend"]
        middleware = []

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/api"

        # Backend references target but overrides authentication
        [backends.api_backend]
        service = "http"
        target_ref = "external-api"
        authentication = "backend-auth"

        [services.http]
        module = ""
    "#;

    let mut config = load_config_from_str(toml).expect("valid config");
    
    harmony::config::resolution::resolve_references(&mut config).expect("resolution should succeed");

    // Verify backend has its own authentication, not target's
    let backend = config.backends.get("api_backend").expect("backend exists");
    assert_eq!(backend.authentication, Some("backend-auth".to_string()));

    // Verify the correct authentication was resolved
    if let Some(options) = &backend.options {
        if let Some(auth_def_json) = options.get("authentication_def") {
            let auth_def: harmony::models::connection::AuthenticationDefinition = 
                serde_json::from_value(auth_def_json.clone()).expect("should deserialize");
            
            assert_eq!(auth_def.id, "backend-auth");
            assert_eq!(
                auth_def.options.get("token").and_then(|v| v.as_str()),
                Some("backend-token")
            );
        }
    }
}

#[test]
fn backend_with_no_authentication() {
    let toml = r#"
        [proxy]
        id = "backend-auth-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.api-proxy]
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["api_backend"]
        middleware = []

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/api"

        # Backend with no authentication
        [backends.api_backend]
        service = "http"
        [backends.api_backend.options]
        base_url = "https://api.example.com"

        [services.http]
        module = ""
    "#;

    let mut config = load_config_from_str(toml).expect("valid config");
    
    harmony::config::resolution::resolve_references(&mut config).expect("resolution should succeed");

    // Verify backend has no authentication
    let backend = config.backends.get("api_backend").expect("backend exists");
    assert_eq!(backend.authentication, None);

    // Verify no authentication_def was injected
    if let Some(options) = &backend.options {
        assert!(!options.contains_key("authentication_def"));
    }
}
