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

fn get_test_config() -> &'static str {
    r#"
        [proxy]
        id = "smoke-test"
        log_level = "info"
        store_dir = "./tmp"

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
    "#
}

async fn build_test_router() -> axum::Router<()> {
    // Ensure ./tmp directory exists for store_dir
    let _ = std::fs::create_dir_all("./tmp");

    let cfg = load_config_from_str(get_test_config()).expect("valid config");
    harmony::router::build_network_router(Arc::new(cfg), "default").await
}

#[tokio::test]
async fn smoke_http_get_echo_backend() {
    let app = build_test_router().await;

    // Drive a GET through the http endpoint which should pass through middleware, backend echo, and back
    let response = app
        .oneshot(
            Request::builder()
                .uri("/smoke/ping")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Verify body JSON contains echo backend marker and correct subpath
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");
    assert_eq!(json["path"], "ping");
    assert_eq!(json["full_path"], "/smoke/ping");
    assert!(json["headers"].is_object());
}

#[tokio::test]
async fn smoke_http_post_echo_backend() {
    let app = build_test_router().await;

    let test_payload = serde_json::json!({"ping": "pong"});
    let payload_str = serde_json::to_string(&test_payload).expect("serialize payload");

    // Drive a POST through the http endpoint
    let response = app
        .oneshot(
            Request::builder()
                .uri("/smoke/echo")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Verify body JSON contains echo backend marker and correct subpath
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");
    assert_eq!(json["path"], "echo");
    assert_eq!(json["full_path"], "/smoke/echo");
    assert!(json["headers"].is_object());
}

#[tokio::test]
async fn smoke_http_put_echo_backend() {
    let app = build_test_router().await;

    let test_payload = serde_json::json!({"update": "data"});
    let payload_str = serde_json::to_string(&test_payload).expect("serialize payload");

    // Drive a PUT through the http endpoint
    let response = app
        .oneshot(
            Request::builder()
                .uri("/smoke/resource/123")
                .method("PUT")
                .header("content-type", "application/json")
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Verify body JSON contains echo backend marker and correct subpath
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");
    assert_eq!(json["path"], "resource/123");
    assert_eq!(json["full_path"], "/smoke/resource/123");
    assert!(json["headers"].is_object());
}

#[tokio::test]
async fn smoke_http_delete_echo_backend() {
    let app = build_test_router().await;

    // Drive a DELETE through the http endpoint
    let response = app
        .oneshot(
            Request::builder()
                .uri("/smoke/resource/123")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Verify body JSON contains echo backend marker and correct subpath
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");
    assert_eq!(json["path"], "resource/123");
    assert_eq!(json["full_path"], "/smoke/resource/123");
    assert!(json["headers"].is_object());
}
