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
[authentications.other_basic.options]
username = "svc"
password = "pw"

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

/// References and names come from the config file, and the error goes to
/// stderr and the reload log. A newline or an ANSI escape in them must not
/// start a new log line or reach the terminal. Control characters are printed
/// escaped, in the reference, the item name, the prefix hint and the list of
/// available keys.
#[test]
fn control_characters_in_authentication_references_are_escaped() {
    let toml = config_with(&format!(
        r#"
[authentications."a\nb"]
id = "a_b"
method = "basic"
[authentications."a\nb".options]
username = "svc"
password = "pw"

[targets.upstream]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "x\nERROR: forged \u001b[31mline"

[targets."t\u001bx"]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "authentications.b\u001bc"
{}"#,
        BACKEND_VIA_TARGET
    ));

    let err = load_via_file(&toml).expect_err("unknown authentication must fail load");

    let expected = [
        r#"Target 't\u{1b}x' references unknown authentication 'authentications.b\u{1b}c'. Drop the "authentications." prefix: use "b\u{1b}c" instead of "authentications.b\u{1b}c". Available authentications: a\nb, fhir_bearer, other_basic"#,
        r"Target 'upstream' references unknown authentication 'x\nERROR: forged \u{1b}[31mline'. Available authentications: a\nb, fhir_bearer, other_basic",
    ]
    .join("\n");
    assert!(
        err.ends_with(&format!(": {}", expected)),
        "unexpected error.\nexpected tail:\n{}\nactual:\n{}",
        expected,
        err
    );
    assert!(!err.contains('\u{1b}'), "raw ESC in error: {:?}", err);
    assert_eq!(err.lines().count(), 2, "forged line in error: {:?}", err);
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

// ---------------------------------------------------------------------------
// Authentication definitions that cannot produce a credential must fail
// config load (AURA-2437). Before this, they loaded and every request went
// upstream without a credential.
// ---------------------------------------------------------------------------

const BACKEND_WITH_FHIR_BEARER: &str = r#"
[backends.api_backend]
service = "http"
authentication = "fhir_bearer"
[backends.api_backend.options]
base_url = "https://api.example.com"
"#;

#[test]
fn incomplete_authentication_definitions_fail_load() {
    let cases = [
        (
            "method = \"bearer\"",
            "method 'bearer' requires a non-empty 'token' option",
        ),
        (
            "method = \"bearer\"\noptions = { token = \"\" }",
            "method 'bearer' requires a non-empty 'token' option",
        ),
        (
            "method = \"basic\"\noptions = { username = \"svc\" }",
            "method 'basic' requires a non-empty 'password' option",
        ),
        (
            "method = \"basic\"\noptions = { password = \"pw\" }",
            "method 'basic' requires a non-empty 'username' option",
        ),
        (
            "method = \"api_key\"\noptions = { header_name = \"X-Token\" }",
            "method 'api_key' requires a non-empty 'api_key' option",
        ),
        (
            "method = \"api_key\"\noptions = { api_key = \"k\", header_name = \"X Token\" }",
            "'header_name' value 'X Token' is not a valid HTTP header name",
        ),
        ("method = \"digest\"", "unknown method 'digest'"),
        ("method = \"\"", "unknown method ''"),
    ];
    for (definition, expected) in cases {
        // Nothing references `broken`: an unused definition is still checked.
        let toml = config_with(&format!(
            "[authentications.broken]\nid = \"broken\"\n{}\n{}",
            definition, BACKEND_WITH_FHIR_BEARER
        ));
        let err = load_via_file(&toml).expect_err(definition);
        assert!(
            err.contains("Authentication 'broken' is invalid") && err.contains(expected),
            "{:?} should fail with {:?}, got: {}",
            definition,
            expected,
            err
        );
    }
}

#[test]
fn every_invalid_definition_is_reported_in_one_load() {
    let toml = config_with(&format!(
        r#"
[authentications.no_token]
id = "no_token"
method = "bearer"

[authentications.no_password]
id = "no_password"
method = "basic"
options = {{ username = "svc" }}
{}"#,
        BACKEND_WITH_FHIR_BEARER
    ));
    let err = load_via_file(&toml).expect_err("both definitions are invalid");
    assert!(
        err.contains("Authentication 'no_password' is invalid"),
        "{}",
        err
    );
    assert!(
        err.contains("Authentication 'no_token' is invalid"),
        "{}",
        err
    );
}

#[test]
fn invalid_definition_error_escapes_control_characters_in_key() {
    let toml = config_with(&format!(
        "[authentications.\"x\\nERROR: forged\"]\nid = \"x\"\nmethod = \"bearer\"\n{}",
        BACKEND_WITH_FHIR_BEARER
    ));
    let err = load_via_file(&toml).expect_err("definition has no token");
    assert!(
        err.contains(r"Authentication 'x\nERROR: forged' is invalid"),
        "key should be escaped: {}",
        err
    );
    assert!(
        !err.contains("\nERROR: forged"),
        "raw newline must not reach the message: {}",
        err
    );
}

#[test]
fn jwt_definition_is_rejected_for_targets_and_backends() {
    let jwt = r#"
[authentications.svc_jwt]
id = "svc_jwt"
method = "jwt"
options = { use_hs256 = true, hs256_secret = "s" }
"#;
    let via_backend = config_with(&format!(
        r#"{}
[backends.api_backend]
service = "http"
authentication = "svc_jwt"
[backends.api_backend.options]
base_url = "https://api.example.com"
"#,
        jwt
    ));
    let err = load_via_file(&via_backend).expect_err("a backend cannot send a jwt definition");
    assert!(
        err.contains("Backend 'api_backend' references authentication 'svc_jwt' with method 'jwt', which a backend cannot send"),
        "{}",
        err
    );

    let via_target = config_with(&format!(
        r#"{}
[targets.upstream]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "svc_jwt"
{}"#,
        jwt, BACKEND_VIA_TARGET
    ));
    let err = load_via_file(&via_target).expect_err("a target cannot send a jwt definition");
    assert!(
        err.contains("Target 'upstream' references authentication 'svc_jwt' with method 'jwt', which a target cannot send"),
        "{}",
        err
    );
}

#[test]
fn jwt_definition_is_accepted_for_middleware() {
    let toml = config_with(&format!(
        r#"
[authentications.svc_jwt]
id = "svc_jwt"
method = "jwt"
options = {{ use_hs256 = true, hs256_secret = "s", issuer = "https://auth.example.com/", audience = "api", public_key_path = "" }}

[middleware.jwt_check]
type = "jwt_auth"
authentication = "svc_jwt"
{}"#,
        BACKEND_WITH_FHIR_BEARER
    ));
    load_via_file(&toml).expect("middleware may reference a jwt definition");
}

#[test]
fn inline_authentication_def_in_options_fails_load() {
    let inline = r#"authentication_def = { id = "x", method = "bearer", options = {} }"#;
    let cases = [
        (
            format!(
                "[backends.api_backend]\nservice = \"http\"\n[backends.api_backend.options]\nbase_url = \"https://api.example.com\"\n{}\n",
                inline
            ),
            "Backend 'api_backend' sets 'authentication_def' in its options",
        ),
        (
            format!(
                "{}\n[middleware.hook]\ntype = \"webhook\"\n[middleware.hook.options]\nendpoint = \"https://hooks.example.com\"\n{}\n",
                BACKEND_WITH_FHIR_BEARER, inline
            ),
            "Middleware 'hook' sets 'authentication_def' in its options",
        ),
    ];
    for (extra, expected) in cases {
        let err = load_via_file(&config_with(&extra)).expect_err(expected);
        assert!(
            err.contains(expected) && err.contains("Define the credential under [authentications]"),
            "expected {:?}, got: {}",
            expected,
            err
        );
    }
}

#[test]
fn inline_authentication_def_in_endpoint_options_fails_load() {
    // config_with already defines [endpoints.http_in], so this builds its own
    // endpoint table.
    let toml = config_with(&format!(
        r#"{}
[endpoints.other_in]
service = "http"
[endpoints.other_in.options]
path_prefix = "/other"
authentication_def = {{ id = "x", method = "bearer", options = {{}} }}
"#,
        BACKEND_WITH_FHIR_BEARER
    ));
    let err = load_via_file(&toml).expect_err("inline definition on an endpoint");
    assert!(
        err.contains("Endpoint 'other_in' sets 'authentication_def' in its options"),
        "{}",
        err
    );
}

/// Writes the main config and extra files (relative path, contents) into a
/// temp dir, then loads the main config the way the binary does.
fn load_files(main: &str, extra_files: &[(&str, &str)]) -> Result<Config, String> {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    for (relative, contents) in extra_files {
        let path = dir.path().join(relative);
        std::fs::create_dir_all(path.parent().expect("file has a parent")).expect("create dir");
        std::fs::write(&path, contents).expect("write extra config");
    }
    let path = dir.path().join("config.toml");
    std::fs::write(&path, main).expect("write config");
    Config::load(path.to_str().expect("utf-8 path")).map_err(|e| match e {
        ConfigError::InvalidProxy { reason, .. } => reason,
        other => format!("{:?}", other),
    })
}

const PIPELINE_FILE_AUTH: &str = r#"
[authentications.pipeline_bearer]
id = "pipeline_bearer"
method = "bearer"
[authentications.pipeline_bearer.options]
token = "from-pipeline-file"
"#;

#[test]
fn authentication_defined_in_a_pipeline_file_is_merged() {
    let main = config_with(
        r#"
[backends.api_backend]
service = "http"
authentication = "pipeline_bearer"
[backends.api_backend.options]
base_url = "https://api.example.com"
"#,
    );
    let config = load_files(&main, &[("pipelines/auth.toml", PIPELINE_FILE_AUTH)])
        .expect("a definition in a pipeline file must resolve");

    let options = config.backends["api_backend"]
        .options
        .as_ref()
        .expect("backend options");
    assert_eq!(
        options["authentication_def"]["options"]["token"],
        serde_json::json!("from-pipeline-file")
    );
}

#[test]
fn authentication_defined_in_two_files_fails_load() {
    let main = config_with(BACKEND_WITH_FHIR_BEARER);
    let err = load_files(
        &main,
        &[
            ("pipelines/auth.toml", PIPELINE_FILE_AUTH),
            ("mesh/auth.toml", PIPELINE_FILE_AUTH),
        ],
    )
    .expect_err("a key defined twice must fail load");
    assert!(
        err.contains("Authentication 'pipeline_bearer' is defined in both"),
        "{}",
        err
    );
    assert!(
        err.contains("auth.toml"),
        "error should name the files: {}",
        err
    );
}

#[test]
fn authentication_in_a_file_that_duplicates_the_main_config_fails_load() {
    let main = config_with(BACKEND_WITH_FHIR_BEARER);
    let duplicate = r#"
[authentications.fhir_bearer]
id = "fhir_bearer"
method = "bearer"
[authentications.fhir_bearer.options]
token = "other"
"#;
    let err = load_files(&main, &[("transforms/auth.toml", duplicate)])
        .expect_err("a file must not replace a main-config credential");
    assert!(
        err.contains("Authentication 'fhir_bearer' is defined in both")
            && err.contains("config.toml")
            && err.contains("transforms/auth.toml"),
        "{}",
        err
    );
}

#[test]
fn jwt_definition_is_rejected_for_webhook_middleware() {
    let toml = config_with(&format!(
        r#"
[authentications.svc_jwt]
id = "svc_jwt"
method = "jwt"
options = {{ use_hs256 = true, hs256_secret = "s" }}

[middleware.hook]
type = "webhook"
authentication = "svc_jwt"
[middleware.hook.options]
endpoint = "https://hooks.example.com"
{}"#,
        BACKEND_WITH_FHIR_BEARER
    ));
    let err = load_via_file(&toml).expect_err("a webhook cannot send a jwt definition");
    assert!(
        err.contains("Middleware 'hook' references authentication 'svc_jwt' with method 'jwt', which a webhook cannot send"),
        "{}",
        err
    );
}

#[test]
fn token_with_trailing_newline_fails_load_without_echoing_it() {
    let toml = config_with(&format!(
        "[authentications.newline]\nid = \"newline\"\nmethod = \"bearer\"\noptions = {{ token = \"s3cret\\n\" }}\n{}",
        BACKEND_WITH_FHIR_BEARER
    ));
    let err = load_via_file(&toml).expect_err("a newline cannot go into a header");
    assert!(
        err.contains("Authentication 'newline' is invalid")
            && err.contains("option 'token' contains characters that are not allowed"),
        "{}",
        err
    );
    assert!(
        !err.contains("s3cret"),
        "the secret must not be echoed: {}",
        err
    );
}

#[test]
fn errors_from_different_checks_are_reported_in_one_load() {
    let toml = config_with(
        r#"
[authentications.no_token]
id = "no_token"
method = "bearer"

[backends.api_backend]
service = "http"
authentication = "missing_auth"
[backends.api_backend.options]
base_url = "https://api.example.com"
authentication_def = { id = "x", method = "bearer", options = {} }
"#,
    );
    let err = load_via_file(&toml).expect_err("three separate problems");
    assert!(
        err.contains("Authentication 'no_token' is invalid"),
        "{}",
        err
    );
    assert!(
        err.contains("Backend 'api_backend' references unknown authentication 'missing_auth'"),
        "{}",
        err
    );
    assert!(
        err.contains("Backend 'api_backend' sets 'authentication_def' in its options"),
        "{}",
        err
    );
}

#[test]
fn overlapping_folder_settings_do_not_report_a_file_as_its_own_duplicate() {
    // transforms_path names the pipelines folder, so the same file is read twice.
    let main = config_with(
        r#"
[backends.api_backend]
service = "http"
authentication = "pipeline_bearer"
[backends.api_backend.options]
base_url = "https://api.example.com"
"#,
    )
    .replace(
        "id = \"auth-ref-test\"",
        "id = \"auth-ref-test\"\ntransforms_path = \"pipelines\"",
    );
    load_files(&main, &[("pipelines/auth.toml", PIPELINE_FILE_AUTH)])
        .expect("reading one file twice is not a duplicate definition");
}
