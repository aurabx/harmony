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

#[tokio::test]
async fn http_to_dicom_backend_echo_succeeds() {
    let toml = r#"
        [proxy]
        id = "dicom-backend-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.bridge]
        description = "HTTP -> DICOM backend bridge"
        networks = ["default"]
        endpoints = ["http_to_dicom"]
        backends = ["dicom_pacs"]
        middleware = []

        [endpoints.http_to_dicom]
        service = "http"
        [endpoints.http_to_dicom.options]
        path_prefix = "/dicom"

        [backends.dicom_pacs]
        service = "dicom"

        [backends.dicom_pacs.options]
        aet = "ORTHANC"
        host = "localhost"
        port = 4242
        local_aet = "HARMONY_SCU"

        [services.http]
        module = ""
        [services.dicom]
        module = ""
    "#;

    // Start a DCMTK storescp to respond to C-ECHO on 4242
    // If dcmtk is not installed, this test will fail. Ensure `brew install dcmtk`.
    std::fs::create_dir_all("../../tmp/dcmtk_in").expect("create dcmtk output dir");
    let mut child = tokio::process::Command::new("storescp")
        .arg("--fork")
        .arg("--aetitle")
        .arg("ORTHANC")
        .arg("--output-directory")
        .arg("./tmp/dcmtk_in")
        .arg("4242")
        .kill_on_drop(true)
        .spawn()
        .expect("spawn storescp");

    // Wait for port to accept connections
    for _ in 0..30 {
        if tokio::net::TcpStream::connect("127.0.0.1:4242")
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let cfg: Config = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // POST /dicom/echo should trigger SCU echo in the DICOM backend and return a JSON body
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicom/echo")
                .method("POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");

    let json: serde_json::Value = serde_json::from_slice(&body).expect("parse json body");
    assert_eq!(json.get("operation").and_then(|v| v.as_str()), Some("echo"));
    assert_eq!(json.get("success").and_then(|v| v.as_bool()), Some(true));

    // Cleanup DCMTK process
    let _ = child.kill().await;
}
