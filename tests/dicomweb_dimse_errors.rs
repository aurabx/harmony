//! End-to-end DIMSE failures through the DICOMweb to DIMSE bridge (AURA-2566).
//!
//! The `dicom_scu` backend points at a closed local port, so every C-FIND and
//! C-GET fails. The client must get 502 Bad Gateway with no backend detail.
//! The raw SCU error names the host, port and AE title, and belongs in the
//! server log only.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const BACKEND_AET: &str = "HIDDEN_PACS_AET";
const LOCAL_AET: &str = "HIDDEN_SCU_AET";

/// A local port with nothing listening on it.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    let port = listener.local_addr().expect("local address").port();
    drop(listener);
    port
}

fn bridge_config(pacs_port: u16) -> Arc<Config> {
    let toml = format!(
        r#"
        [proxy]
        id = "dicomweb-dimse-errors-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8087

        [pipelines.bridge]
        description = "DICOMweb -> DIMSE with an unreachable PACS"
        networks = ["default"]
        endpoints = ["dicomweb"]
        middleware = ["dicomweb_bridge"]
        backends = ["pacs"]

        [endpoints.dicomweb]
        service = "dicomweb"
        [endpoints.dicomweb.options]
        path_prefix = "/dicomweb"

        [backends.pacs]
        service = "dicom_scu"
        [backends.pacs.options]
        aet = "{BACKEND_AET}"
        host = "127.0.0.1"
        port = {pacs_port}
        local_aet = "{LOCAL_AET}"

        [services.dicomweb]
        module = ""
        [services.dicom]
        module = ""

        [middleware.dicomweb_bridge]
        type = "dicomweb_bridge"

        [middleware_types.dicomweb_bridge]
        module = ""
    "#
    );
    let config: Config = toml::from_str(&toml).expect("TOML parse error");
    config.validate().expect("valid config");
    Arc::new(config)
}

/// Send a GET for `uri` and return the status and body text.
async fn get(config: Arc<Config>, uri: &str, accept: &str) -> (StatusCode, String) {
    harmony::globals::set_config(config.clone());
    let app = harmony::router::build_network_router(config, "default").await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("accept", accept)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn assert_no_backend_detail(body: &str, port: u16) {
    for secret in ["127.0.0.1", &port.to_string(), BACKEND_AET, LOCAL_AET] {
        assert!(
            !body.contains(secret),
            "response body leaks '{}': {}",
            secret,
            body
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn qido_against_unreachable_pacs_is_502_without_detail() {
    let port = closed_port();
    let (status, body) = get(
        bridge_config(port),
        "/dicomweb/studies",
        "application/dicom+json",
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "body: {}", body);
    assert_no_backend_detail(&body, port);
}

#[tokio::test]
#[serial_test::serial]
async fn wado_instance_against_unreachable_pacs_is_502_without_detail() {
    let port = closed_port();
    let (status, body) = get(
        bridge_config(port),
        "/dicomweb/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5",
        "multipart/related; type=\"application/dicom\"",
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "body: {}", body);
    assert_no_backend_detail(&body, port);
}
