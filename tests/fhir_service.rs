#![cfg(test)]

use std::sync::Arc;
use axum::Router;
use http::{Method, Request, StatusCode};
use tower::ServiceExt; // for `oneshot`
use harmony::config::config::Config;
use harmony::router::build_network_router;

fn load_config_from_str(toml_str: &str) -> Config {
    let cfg: Config = toml::from_str(toml_str).expect("TOML parse error");
    cfg.validate().expect("config should validate");
    cfg
}

#[tokio::test]
async fn fhir_endpoint_handles_get_request() {
    // Ensure ./tmp exists per project preference
    let _ = std::fs::create_dir_all("./tmp");

    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = ["fhir"]
        backends = []
        middleware = []

        [endpoints.fhir]
        service = "fhir"
        [endpoints.fhir.options]
        path_prefix = "/fhir"

        [services.fhir]
        module = ""
    "#;

    let cfg = load_config_from_str(toml);
    let router: Router<()> = build_network_router(Arc::new(cfg), "default").await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("/fhir/patient/123")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("router should respond");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(json["message"], "FHIR endpoint received the request");
    assert_eq!(json["path"], "patient/123");
    assert_eq!(json["full_path"], "/fhir/patient/123");
    assert!(json["headers"].is_object());
}

#[tokio::test]
async fn fhir_put_is_supported() {
    // Ensure ./tmp exists per project preference
    let _ = std::fs::create_dir_all("./tmp");

    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = ["fhir"]
        backends = []
        middleware = []

        [endpoints.fhir]
        service = "fhir"
        [endpoints.fhir.options]
        path_prefix = "/fhir"

        [services.fhir]
        module = ""
    "#;

    let cfg = load_config_from_str(toml);
    let router: Router<()> = build_network_router(Arc::new(cfg), "default").await;

    let req = Request::builder()
        .method(Method::PUT)
        .uri("/fhir/Patient/123")
        .body(axum::body::Body::from("{\"resourceType\":\"Patient\"}"))
        .unwrap();

    let resp = router.oneshot(req).await.expect("router should respond");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(json["message"], "FHIR endpoint received the request");
    assert_eq!(json["path"], "Patient/123");
    assert_eq!(json["full_path"], "/fhir/Patient/123");
    assert!(json["headers"].is_object());
}

#[tokio::test]
async fn fhir_delete_is_supported() {
    // Ensure ./tmp exists per project preference
    let _ = std::fs::create_dir_all("./tmp");

    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "Core pipeline"
        networks = ["default"]
        endpoints = ["fhir"]
        backends = []
        middleware = []

        [endpoints.fhir]
        service = "fhir"
        [endpoints.fhir.options]
        path_prefix = "/fhir"

        [services.fhir]
        module = ""
    "#;

    let cfg = load_config_from_str(toml);
    let router: Router<()> = build_network_router(Arc::new(cfg), "default").await;

    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/fhir/Patient/123")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("router should respond");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(json["message"], "FHIR endpoint received the request");
    assert_eq!(json["path"], "Patient/123");
    assert_eq!(json["full_path"], "/fhir/Patient/123");
    assert!(json["headers"].is_object());
}

#[tokio::test]
async fn fhir_backend_is_invoked_in_pipeline() {
    // Ensure ./tmp exists per project preference
    let _ = std::fs::create_dir_all("./tmp");

    let toml = r#"
        [proxy]
        id = "router-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "FHIR passthrough"
        networks = ["default"]
        endpoints = ["fhir_endpoint"]
        backends = ["fhir_backend"]
        middleware = []

        [endpoints.fhir_endpoint]
        service = "fhir"
        [endpoints.fhir_endpoint.options]
        path_prefix = "/fhir"

        [backends.fhir_backend]
        service = "fhir"
        [backends.fhir_backend.options]
        path_prefix = "/fhir"

        [services.fhir]
        module = ""
    "#;

    let cfg = load_config_from_str(toml);
    let router: Router<()> = build_network_router(Arc::new(cfg), "default").await;

    let req = Request::builder()
        .method(Method::POST)
        .uri("/fhir/Observation")
        .body(axum::body::Body::from("{\"resourceType\":\"Observation\"}"))
        .unwrap();

    let resp = router.oneshot(req).await.expect("router should respond");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(json["message"], "FHIR endpoint received the request");
    assert_eq!(json["path"], "Observation");
    assert_eq!(json["full_path"], "/fhir/Observation");
    assert!(json["headers"].is_object());
}
