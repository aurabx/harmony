use harmony::config::config::{Config, ConfigError};
use axum::http::{Request, StatusCode};
use axum::body::Body;
use tower::ServiceExt; // for Router::oneshot
use std::sync::Arc;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn smoke_http_frontend_echo_backend_with_test_middleware() {
    let toml = r#"
        [proxy]
        id = "smoke-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "HTTP->Echo smoke pipeline"
        networks = ["default"]
        endpoints = ["smoke_http"]
        backends = ["echo_backend"]
        middleware = ["middleware.passthru"]

        [endpoints.smoke_http]
        service = "http"
        [endpoints.smoke_http.options]
        path_prefix = "/smoke"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo-back"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        # Register passthru as built-in
        [middleware_types.passthru]
        module = ""
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");

    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Drive a GET through the http endpoint which should pass through middleware, backend echo, and back
    let response = app
        .oneshot(
            Request::builder()
                .uri("/smoke/get-route")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Verify body contains echo backend marker (indirectly confirming the backend ran)
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    assert!(body_str.contains("Echo endpoint received the request"));
}
