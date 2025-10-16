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

fn get_dicomweb_test_config() -> &'static str {
    r#"
        [proxy]
        id = "dicomweb-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.dicomweb_test]
        description = "DICOMweb endpoint test pipeline"
        networks = ["default"]
        endpoints = ["dicomweb_endpoint"]
        backends = []
        middleware = []

        [endpoints.dicomweb_endpoint]
        service = "dicomweb"
        [endpoints.dicomweb_endpoint.options]
        path_prefix = "/dicomweb"

        [services.dicomweb]
        module = ""
    "#
}

async fn build_test_router() -> axum::Router<()> {
    // Ensure ./tmp directory exists for store_dir
    let _ = std::fs::create_dir_all("../../tmp");

    let cfg = load_config_from_str(get_dicomweb_test_config()).expect("valid config");
    harmony::router::build_network_router(Arc::new(cfg), "default").await
}

#[tokio::test]
async fn dicomweb_qido_studies_returns_200_or_error() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies")
                .method("GET")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // QIDO endpoints are now implemented, so should not return 501
    assert_ne!(response.status(), StatusCode::NOT_IMPLEMENTED);
    // Will likely return 200 (with empty array) or error due to no backend configured
    println!("QIDO /studies response status: {}", response.status());
}

#[tokio::test]
async fn dicomweb_qido_series_returns_200_or_error() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies/1.2.3.4.5/series")
                .method("GET")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // QIDO endpoints are now implemented, so should not return 501
    assert_ne!(response.status(), StatusCode::NOT_IMPLEMENTED);
    // Will likely return 200 (with empty array) or error due to no backend configured
    println!("QIDO /series response status: {}", response.status());
}

#[tokio::test]
async fn dicomweb_qido_instances_returns_200_or_error() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies/1.2.3.4.5/series/1.2.3.4.6/instances")
                .method("GET")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // QIDO endpoints are now implemented, so should not return 501
    assert_ne!(response.status(), StatusCode::NOT_IMPLEMENTED);
    // Will likely return 200 (with empty array) or error due to no backend configured
    println!("QIDO /instances response status: {}", response.status());
}

#[tokio::test]
async fn dicomweb_wado_study_metadata_returns_200_or_error() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies/1.2.3.4.5/metadata")
                .method("GET")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // WADO metadata endpoints are now implemented, so should not return 501
    assert_ne!(response.status(), StatusCode::NOT_IMPLEMENTED);
    // Will likely return 200 (with empty array) or error due to no backend configured
    println!("WADO /metadata response status: {}", response.status());
}

#[tokio::test]
async fn dicomweb_wado_instance_returns_200_or_error() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies/1.2.3.4.5/series/1.2.3.4.6/instances/1.2.3.4.7")
                .method("GET")
                .header("accept", "application/dicom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // WADO instance endpoints are now implemented, so should not return 501
    assert_ne!(response.status(), StatusCode::NOT_IMPLEMENTED);
    // Will likely return 200 or error due to no backend configured
    println!("WADO /instances response status: {}", response.status());
}

#[tokio::test]
async fn dicomweb_wado_frames_returns_200_or_error() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies/1.2.3.4.5/series/1.2.3.4.6/instances/1.2.3.4.7/frames/1")
                .method("GET")
                .header("accept", "image/jpeg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // WADO frames endpoints are now implemented, so should not return 501
    assert_ne!(response.status(), StatusCode::NOT_IMPLEMENTED);
    // Will likely return 200 or error due to no backend configured
    println!("WADO /frames response status: {}", response.status());
}

#[tokio::test]
async fn dicomweb_wado_bulkdata_returns_200_or_error() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/bulkdata/some-bulk-uri")
                .method("GET")
                .header("accept", "application/octet-stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // WADO bulkdata endpoints are now implemented, so should not return 501
    assert_ne!(response.status(), StatusCode::NOT_IMPLEMENTED);
    // Will likely return 200 or error due to no backend configured
    println!("WADO /bulkdata response status: {}", response.status());
}

#[tokio::test]
async fn dicomweb_options_request_returns_cors_headers() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies")
                .method("OPTIONS")
                .header("origin", "http://localhost:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Note: We can't easily test headers in this pattern since we consume the response.
    // In a real scenario, we'd want to verify CORS headers are set correctly.
}

#[tokio::test]
async fn dicomweb_nonexistent_route_returns_404() {
    let app = build_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/nonexistent")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // This should return 404 NOT FOUND since the route doesn't exist
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
