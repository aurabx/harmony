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

// ---------------------------------------------------------------------------
// Unresolved authentication references must fail config load (AURA-2434).
// These go through `Config::load`, the path used at startup, by
// `--validate-config` and by hot reload.
// ---------------------------------------------------------------------------

/// Base config with one authentication (`fhir_bearer`) and a pipeline whose
/// backend inherits from the `upstream` target. `extra` is appended verbatim.
fn config_with(extra: &str) -> String {
    format!(
        r#"
[proxy]
id = "auth-ref-test"

[management]
enabled = false

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080

[authentications.fhir_bearer]
id = "fhir_bearer"
method = "bearer"
[authentications.fhir_bearer.options]
token = "t"

[authentications.other_basic]
id = "other_basic"
method = "basic"

[pipelines.core]
networks = ["default"]
endpoints = ["http_in"]
backends = ["api_backend"]
middleware = []

[endpoints.http_in]
service = "http"
[endpoints.http_in.options]
path_prefix = "/api"

[services.http]
module = ""

{}
"#,
        extra
    )
}

/// Writes `toml` to a temp dir and loads it the way the binary does.
fn load_via_file(toml: &str) -> Result<Config, String> {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, toml).expect("write config");
    Config::load(path.to_str().expect("utf-8 path")).map_err(|e| match e {
        ConfigError::InvalidProxy { reason, .. } => reason,
        other => format!("{:?}", other),
    })
}

const BACKEND_VIA_TARGET: &str = r#"
[backends.api_backend]
service = "http"
target_ref = "upstream"
"#;

#[test]
fn target_with_unknown_authentication_fails_load() {
    let toml = config_with(&format!(
        r#"
[targets.upstream]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "fhir_bearr"
{}"#,
        BACKEND_VIA_TARGET
    ));

    let err = load_via_file(&toml).expect_err("unknown target authentication must fail load");
    assert!(
        err.contains("Target 'upstream' references unknown authentication 'fhir_bearr'"),
        "error should name the target and the missing key: {}",
        err
    );
    assert!(
        err.contains("Available authentications: fhir_bearer, other_basic"),
        "error should list the available keys: {}",
        err
    );
    assert!(!err.contains("prefix"), "bare key needs no prefix hint: {}", err);
}

#[test]
fn backend_with_unknown_authentication_fails_load() {
    let toml = config_with(
        r#"
[backends.api_backend]
service = "http"
authentication = "missing_auth"
[backends.api_backend.options]
base_url = "https://api.example.com"
"#,
    );

    let err = load_via_file(&toml).expect_err("unknown backend authentication must fail load");
    assert!(
        err.contains("Backend 'api_backend' references unknown authentication 'missing_auth'"),
        "error should name the backend and the missing key: {}",
        err
    );
    assert!(
        err.contains("Available authentications: fhir_bearer, other_basic"),
        "error should list the available keys: {}",
        err
    );
}

#[test]
fn peer_with_unknown_authentication_fails_load() {
    let toml = config_with(
        r#"
[backends.api_backend]
service = "http"
[backends.api_backend.options]
base_url = "https://api.example.com"

[peers.hospital_a]
connection.host = "10.0.0.1"
connection.port = 11112
connection.protocol = "dicom"
authentication = "missing_auth"
"#,
    );

    let err = load_via_file(&toml).expect_err("unknown peer authentication must fail load");
    assert!(
        err.contains("Peer 'hospital_a' references unknown authentication 'missing_auth'"),
        "error should name the peer and the missing key: {}",
        err
    );
}

#[test]
fn middleware_with_unknown_authentication_fails_load() {
    let toml = config_with(
        r#"
[backends.api_backend]
service = "http"
[backends.api_backend.options]
base_url = "https://api.example.com"

[middleware.jwt_middleware]
type = "jwt_auth"
authentication = "missing_auth"
"#,
    );

    let err = load_via_file(&toml).expect_err("unknown middleware authentication must fail load");
    assert!(
        err.contains("Middleware 'jwt_middleware' references unknown authentication 'missing_auth'"),
        "error should name the middleware and the missing key: {}",
        err
    );
}

#[test]
fn prefixed_authentication_reference_fails_load_with_hint() {
    let toml = config_with(&format!(
        r#"
[targets.upstream]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "authentications.fhir_bearer"
{}"#,
        BACKEND_VIA_TARGET
    ));

    let err = load_via_file(&toml).expect_err("prefixed authentication reference must fail load");
    assert!(
        err.contains("Target 'upstream' references unknown authentication 'authentications.fhir_bearer'"),
        "error should name the target and the reference as written: {}",
        err
    );
    assert!(
        err.contains(r#"use "fhir_bearer" instead of "authentications.fhir_bearer""#),
        "error should tell the user to drop the prefix: {}",
        err
    );
}

#[test]
fn every_unresolved_authentication_reference_is_reported() {
    let toml = config_with(&format!(
        r#"
[targets.upstream]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "authentications.fhir_bearer"

[targets.second]
connection.host = "second.example.com"
connection.protocol = "https"
authentication = "nope"
{}"#,
        BACKEND_VIA_TARGET
    ));

    let err = load_via_file(&toml).expect_err("unresolved references must fail load");
    assert!(err.contains("Target 'upstream'"), "first bad target missing: {}", err);
    assert!(err.contains("Target 'second'"), "second bad target missing: {}", err);
}

#[test]
fn bare_authentication_reference_loads_and_is_injected() {
    let toml = config_with(&format!(
        r#"
[targets.upstream]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "fhir_bearer"
{}"#,
        BACKEND_VIA_TARGET
    ));

    let config = load_via_file(&toml).expect("bare reference should load");
    let backend = config.backends.get("api_backend").expect("backend exists");
    let auth_def = backend
        .options
        .as_ref()
        .and_then(|options| options.get("authentication_def"))
        .expect("authentication_def should be injected into backend options");
    assert_eq!(auth_def["id"], "fhir_bearer");
    assert_eq!(auth_def["method"], "bearer");
}

/// The binary must exit non-zero and print the reason on stderr, both for
/// `--validate-config` and for a normal start. Config load runs before
/// logging is set up, so stderr is the only place the message can appear.
#[test]
fn binary_exits_non_zero_on_unresolved_authentication() {
    let toml = config_with(&format!(
        r#"
[targets.upstream]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "authentications.fhir_bearer"
{}"#,
        BACKEND_VIA_TARGET
    ));
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, toml).expect("write config");

    for extra_args in [vec!["--validate-config"], vec![]] {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_harmony"))
            .arg("--config")
            .arg(&path)
            .args(&extra_args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run harmony binary");

        // If the config were accepted, a normal start would serve forever.
        // Kill it after a deadline so a regression fails instead of hanging.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while child.try_wait().expect("poll harmony").is_none() {
            if std::time::Instant::now() > deadline {
                child.kill().ok();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let output = child.wait_with_output().expect("collect harmony output");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(1),
            "harmony {:?} should exit 1; stderr: {}",
            extra_args,
            stderr
        );
        assert!(
            stderr.contains(r#"use "fhir_bearer" instead of "authentications.fhir_bearer""#),
            "harmony {:?} should explain the error on stderr: {}",
            extra_args,
            stderr
        );
    }
}
