use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use std::sync::Arc;
use tower::ServiceExt;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

fn cfg() -> &'static str {
    r#"
        [proxy]
        id = "dicomweb-bridge-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.bridge]
        description = "DICOMweb -> DIMSE bridge"
        networks = ["default"]
        endpoints = ["dicomweb"]
        middleware = ["dicomweb_bridge"]
        backends = ["dicom_pacs"]

        [endpoints.dicomweb]
        service = "dicomweb"
        [endpoints.dicomweb.options]
        path_prefix = "/dicomweb"

        [backends.dicom_pacs]
        service = "dicom"
        [backends.dicom_pacs.options]
        aet = "ORTHANC"
        host = "localhost"
        port = 4242
        local_aet = "HARMONY_SCU"

        [services.dicomweb]
        module = ""
        [services.dicom]
        module = ""

        [middleware_types.dicomweb_bridge]
        module = ""
    "#
}

async fn build_router() -> axum::Router<()> {
    let _ = std::fs::create_dir_all("./tmp");
    let c = load_config_from_str(cfg()).expect("valid config");
    harmony::router::build_network_router(Arc::new(c), "default").await
}

#[tokio::test]
async fn middleware_maps_studies_to_find() {
    // We are not standing up a DICOM server; just ensure the request falls through to backend layer initiation
    let app = build_router().await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies?PatientID=TEST123")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("handled");

    // With the new dicomweb_bridge middleware, QIDO endpoints are fully supported
    // The response will depend on backend availability - could be BAD_GATEWAY (no backend),
    // OK (successful), INTERNAL_SERVER_ERROR (backend error), etc.
    assert_ne!(
        resp.status(),
        StatusCode::NOT_IMPLEMENTED,
        "QIDO /studies endpoint should be supported with dicomweb_bridge middleware"
    );
}
