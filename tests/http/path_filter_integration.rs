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
        id = "path-filter-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.path_filter]
        description = "HTTP -> path_filter -> echo"
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["echo_backend"]
        middleware = ["path_filter"]

        [middleware.path_filter]
        type = "path_filter"
        [middleware.path_filter.options]
        rules = [
          { allow = "/allowed" },
          { deny = "/{*rest}" }  # Catch-all deny
        ]

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/filter"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo-back"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware_types.path_filter]
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
async fn test_path_filter_allowed_path_reaches_backend() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/filter/allowed")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");
    // Echo backend should see the subpath after the prefix
    assert_eq!(json["path"], "allowed");
    assert_eq!(json["full_path"], "/filter/allowed");
}

#[tokio::test]
async fn test_path_filter_denied_path_returns_404() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/filter/denied")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Denied path should surface as 404 via PathDenied -> HTTP adapter mapping
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
