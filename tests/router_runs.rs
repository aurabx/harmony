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