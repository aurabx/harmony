#![cfg(test)]

use std::sync::Arc;
use axum::{Router};
use http::{Request, StatusCode, Method};
use tower::ServiceExt; // for `oneshot`
use harmony::config::config::Config;
use harmony::router::build_network_router;

fn load_config_from_str(toml_str: &str) -> Config {
    let mut cfg: Config = toml::from_str(toml_str).expect("TOML parse error");
    // We don't call from_args; validation will resolve built-in services without registry
    cfg.validate().expect("config should validate");
    cfg
}

#[tokio::test]
async fn fhir_endpoint_handles_get_request() {
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
    // Body is a JSON stringified Option<Value>; ensure our message marker appears
    assert!(body.contains("FHIR endpoint received the request"));
}

#[tokio::test]
async fn fhir_backend_is_invoked_in_pipeline() {
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
    // Request goes through endpoint FHIR transform, then backend FHIR transform, then back out.
    // We at least expect the FHIR message marker to be present after transformations.
    assert!(body.contains("FHIR endpoint received the request"));
}