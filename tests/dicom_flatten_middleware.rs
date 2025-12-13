use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn test_dicom_flatten_left_side() {
    let toml = r#"
        [proxy]
        id = "dicom-flatten-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.flatten_left]
        description = "Test DICOM flatten on left side"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["dicom_flatten_left"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/dicom"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware_types.dicom_flatten]
        module = ""

        [middleware.dicom_flatten_left]
        type = "dicom_flatten"
        [middleware.dicom_flatten_left.options]
        apply = "left"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Send DICOM JSON request
    let dicom_json = json!({
        "00100020": {
            "vr": "LO",
            "Value": ["PID123"]
        },
        "00100010": {
            "vr": "PN",
            "Value": [{"Alphabetic": "Doe^John"}]
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicom/query")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(dicom_json.to_string()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_dicom_flatten_right_side() {
    let toml = r#"
        [proxy]
        id = "dicom-flatten-test-right"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8081

        [pipelines.flatten_right]
        description = "Test DICOM flatten on right side"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["dicom_flatten_right"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/dicom"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware_types.dicom_flatten]
        module = ""

        [middleware.dicom_flatten_right]
        type = "dicom_flatten"
        [middleware.dicom_flatten_right.options]
        apply = "right"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicom/result")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_dicom_flatten_both_sides() {
    let toml = r#"
        [proxy]
        id = "dicom-flatten-test-both"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8082

        [pipelines.flatten_both]
        description = "Test DICOM flatten on both sides"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["dicom_flatten_both"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/dicom"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware_types.dicom_flatten]
        module = ""

        [middleware.dicom_flatten_both]
        type = "dicom_flatten"
        [middleware.dicom_flatten_both.options]
        apply = "both"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let dicom_json = json!({
        "00100020": {
            "vr": "LO",
            "Value": ["PID456"]
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicom/query-both")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(dicom_json.to_string()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
}

