use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use std::sync::Arc;
use tower::ServiceExt; // for Router::oneshot

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn incoming_non_auth_middleware_failure_returns_500() {
    // Create a temporary transform spec that will fail at runtime
    let spec_content = r#"[
        {
            "operation": "shift",
            "spec": {
                "nonexistent.deeply.nested.field": "output"
            }
        }
    ]"#;
    
    let temp_file = tempfile::NamedTempFile::new().expect("create temp file");
    std::fs::write(temp_file.path(), spec_content).expect("write spec file");
    
    let toml = format!(r#"
        [proxy]
        id = "middleware-status-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.test_non_auth_error]
        description = "Test non-auth middleware error returns 500"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["failing_transform"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/test"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware_types.transform]
        module = ""

        # Transform middleware that will fail at runtime
        [middleware.failing_transform]
        type = "transform"
        [middleware.failing_transform.options]
        spec_path = "{spec_path}"
        apply = "left"
        fail_on_error = true
    "#, spec_path = temp_file.path().to_string_lossy());

    let cfg = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Send a request that will trigger transform middleware failure
    // The transform will try to access a nonexistent field and fail
    let response = app
        .oneshot(
            Request::builder()
                .uri("/test/trigger-error")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"some": "data"}"#))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // The transform middleware should fail with a non-auth error,
    // which should result in HTTP 500, not 401
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test] 
async fn jwt_auth_failure_still_returns_401() {
    let toml = r#"
        [proxy]
        id = "jwt-auth-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "HTTP->Echo with jwt auth"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["jwt_auth_test"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/jwt"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware_types.jwtauth]
        module = ""

        [middleware.jwt_auth_test]
        type = "jwt_auth"
        [middleware.jwt_auth_test.options]
        use_hs256 = true
        hs256_secret = "test-secret"
        issuer = "https://test-issuer/"
        audience = "harmony"
        leeway_secs = 60
        public_key_path = ""
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Missing auth header - should still return 401
    let response = app
        .oneshot(
            Request::builder()
                .uri("/jwt/get-route")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn basic_auth_failure_still_returns_401() {
    let toml = r#"
        [proxy]
        id = "basic-auth-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "HTTP->Echo with basic auth"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["basic_auth_test"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/auth"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware.basic_auth_test]
        type = "basic_auth"
        [middleware.basic_auth_test.options]
        username = "u1"
        password = "p1"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Missing auth header - should still return 401
    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/get-route")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}