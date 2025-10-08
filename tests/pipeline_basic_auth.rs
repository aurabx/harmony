use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::{engine::general_purpose, Engine as _};
use harmony::config::config::{Config, ConfigError};
use std::sync::Arc;
use tower::ServiceExt; // for Router::oneshot

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

fn basic_header(user: &str, pass: &str) -> String {
    let creds = format!("{}:{}", user, pass);
    format!(
        "Basic {}",
        general_purpose::STANDARD.encode(creds.as_bytes())
    )
}

#[tokio::test]
async fn basic_auth_allows_valid_credentials() {
    let toml = r#"
        [proxy]
        id = "basic-auth-test"
        log_level = "info"
        store_dir = "/tmp"

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
        middleware = ["middleware.auth_sidecar"]

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

        [middleware.auth_sidecar]
        token_path = ""
        username = "u1"
        password = "p1"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/get-route")
                .method("GET")
                .header("Authorization", basic_header("u1", "p1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn basic_auth_rejects_invalid_credentials() {
    let toml = r#"
        [proxy]
        id = "basic-auth-test"
        log_level = "info"
        store_dir = "/tmp"

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
        middleware = ["middleware.auth_sidecar"]

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

        [middleware.auth_sidecar]
        token_path = ""
        username = "u1"
        password = "p1"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Wrong password
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/get-route")
                .method("GET")
                .header("Authorization", basic_header("u1", "wrong"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Missing header
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
