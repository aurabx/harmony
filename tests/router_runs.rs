use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use std::sync::Arc;
use tower::ServiceExt; // for Router::oneshot

// Helper: parse and validate a config from TOML
fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn router_builds_and_handles_404() {
    // Minimal config with one network and one empty pipeline bound to that network.
    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = []
        backends = []
        middleware = []

        [services.http]
        module = ""
    "#;

    let cfg: Config = load_config_from_str(toml).expect("valid config");

    // Build the router for the default network. This should not panic.
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Fire a simple request against root. Since we didn't mount any routes,
    // axum should return 404 NOT FOUND. The important part is that the router
    // runs and responds.
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn router_handles_basic_request() {
    // Minimal config with one network, one pipeline, and one endpoint.
    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = ["basic"]
        backends = []
        middleware = []

        [endpoints.basic]
        service = "http"
        [endpoints.basic.options]
        path_prefix = "/basic"

        [services.http]
        module = ""
    "#;

    let cfg: Config = load_config_from_str(toml).expect("valid config");

    // Build the router for the default network.
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Send a request to the `/basic/get-route` endpoint (based on HttpEndpoint implementation).
    let response = app
        .oneshot(
            Request::builder()
                .uri("/basic/get-route")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Verify the response status is 200 OK.
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn router_selects_correct_endpoint_based_on_path() {
    // Minimal configuration with two endpoints sharing one network.
    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = ["basic", "fhir"]
        backends = []
        middleware = []

        [endpoints.basic]
        service = "http"
        [endpoints.basic.options]
        path_prefix = "/basic"

        [endpoints.fhir]
        service = "fhir"
        [endpoints.fhir.options]
        path_prefix = "/fhir"

        [services.http]
        module = ""
        
        [services.fhir]
        module = ""
    "#;

    // Load and validate configuration
    let cfg = load_config_from_str(toml).expect("valid config");

    // Build the router for the default network
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Test `/basic/get-route` endpoint
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/basic/get-route")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Verify the `/basic` route returns 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    // Test `/fhir/:path` endpoint
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/fhir/patient")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Verify the `/fhir` route returns 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    // Test a non-existent route
    let response = app
        .oneshot(
            Request::builder()
                .uri("/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Verify the `/nonexistent` route returns 404 NOT FOUND
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn router_handles_path_based_routing() {
    // Create a configuration with multiple endpoints
    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = ["echo_one", "echo_two"]
        backends = []
        middleware = []

        [endpoints.echo_one]
        service = "echo"
        [endpoints.echo_one.options]
        path_prefix = "/echo1"

        [endpoints.echo_two]
        service = "echo"
        [endpoints.echo_two.options]
        path_prefix = "/echo2"

        [services.echo]
        module = ""
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");

    // Build the router
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Test first endpoint `/echo1/:path`
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo1/test")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"key":"value1"}"#))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("parse response body as string");
    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");
    assert_eq!(json["path"], "test");

    // Test second endpoint `/echo2/:path`
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo2/test")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"key":"value2"}"#))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("parse response body as string");
    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");
    assert_eq!(json["path"], "test");

    // Test non-existent route
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/nonexistent")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"key":"value"}"#))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
