use harmony::config::Config;
use harmony::config::ConfigError;
use harmony::config::Config as HarmonyConfig;
use axum::http::{Request, StatusCode};
use axum::body::Body;
use tower::ServiceExt; // for Router::oneshot

// Helper: parse and validate a config from TOML
fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn router_builds_and_handles_404() {
    // Minimal config with one network and one empty group bound to that network.
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

        [groups.core]
        description = "Core group"
        networks = ["default"]
        endpoints = []
        backends = []
        peers = []

        [groups.core.middleware]
        incoming = []
        outgoing = []
    "#;

    let cfg: HarmonyConfig = load_config_from_str(toml).expect("valid config");

    // Build the router for the default network. This should not panic.
    let app = harmony::router::build_network_router(&cfg, "default").await;

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
    // Minimal config with one network, one group, and one endpoint.
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

        [groups.core]
        description = "Core group"
        networks = ["default"]
        endpoints = ["basic"]
        backends = []

        [groups.core.middleware]
        incoming = []
        outgoing = []

        [endpoints.basic]
        description = "A basic test endpoint"
        path_prefix = "/basic"
        kind = "basic"
        type = "basic" # Add this field
    "#;

    let cfg: HarmonyConfig = load_config_from_str(toml).expect("valid config");

    // Build the router for the default network.
    let app = harmony::router::build_network_router(&cfg, "default").await;

    // Send a request to the `/basic` endpoint.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/basic")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Verify the response status is 200 OK.
    assert_eq!(response.status(), StatusCode::OK);

    // Use axum::body::to_bytes instead of hyper::body::to_bytes.
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON response");
    assert_eq!(
        body,
        serde_json::json!({
            "status": "success",
            "message": "Basic endpoint responding"
        })
    );
}

#[tokio::test]
async fn router_selects_correct_endpoint_based_on_path() {
    use axum::{body::Body, http::{Request, StatusCode}};
    use tower::ServiceExt; // for Router::oneshot

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

        [groups.core]
        description = "Core group"
        networks = ["default"]
        endpoints = ["basic", "custom"]
        backends = []

        [groups.core.middleware]
        incoming = []
        outgoing = []

        [endpoints.basic]
        description = "A basic endpoint"
        path_prefix = "/basic"
        kind = "basic"
        type = "basic"

        [endpoints.custom]
        description = "A custom endpoint"
        path_prefix = "/custom"
        kind = "custom"
        type = "custom"
    "#;

    // Load and validate configuration
    let cfg = load_config_from_str(toml).expect("valid config");

    // Build the router for the default network
    let app = harmony::router::build_network_router(&cfg, "default").await;

    // Test `/basic` endpoint
    let response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/basic")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Verify the `/basic` route returns 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON response");
    assert_eq!(
        body,
        serde_json::json!({
            "status": "success",
            "message": "Basic endpoint responding"
        })
    );

    // Test `/custom` endpoint
    let response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/custom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Verify the `/custom` route returns 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON response");
    assert_eq!(
        body,
        serde_json::json!({
            "status": "success",
            "message": "Custom endpoint responding"
        })
    );

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
    use axum::{Router, http::{Request, StatusCode}, Json};
    use serde_json::json;
    use tower::ServiceExt; // For `Router::oneshot`

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

        [groups.core]
        description = "Core group"
        networks = ["default"]
        endpoints = ["echo_one", "echo_two"]
        backends = []

        [groups.core.middleware]
        incoming = []
        outgoing = []

        [endpoints.echo_one]
        description = "First echo endpoint"
        path_prefix = "/echo1"
        kind = "echo"
        type = "basic"

        [endpoints.echo_two]
        description = "Second echo endpoint"
        path_prefix = "/echo2"
        kind = "echo"
        type = "basic"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");

    // Build the router
    let app = harmony::router::build_network_router(&cfg, "default").await;

    // Test first endpoint `/echo1`
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo1")
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
    let body: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON response");
    assert_eq!(body, json!({"echo": {"key": "value1"}}));

    // Test second endpoint `/echo2`
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo2")
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
    let body: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON response");
    assert_eq!(body, json!({"echo": {"key":"value2"}}));

    // Test non-existent route
    let response = app.clone() // Cloning here again to avoid move
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