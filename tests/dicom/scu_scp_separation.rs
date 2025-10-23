use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

/// Helper to check if DCMTK tools are available
fn dcmtk_available() -> bool {
    for bin in ["dcmqrscp", "storescu", "findscu", "dcmqridx"].iter() {
        if std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("DCMTK tool {} not found", bin);
            return false;
        }
    }
    true
}

/// Helper to spawn dcmqrscp on a free port
async fn spawn_dcmqrscp(verbose: bool) -> (tokio::process::Child, u16, PathBuf) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let base = PathBuf::from("./tmp/qrscp");
    let dbdir = base.join("qrdb");
    std::fs::create_dir_all(&dbdir).expect("create qr db dir");
    let cfg_path = base.join("dcmqrscp.cfg");

    let abs_db = match std::fs::canonicalize(&dbdir) {
        Ok(p) => p,
        Err(_) => std::env::current_dir().unwrap().join(&dbdir),
    };

    let cfg = format!(
        "# Minimal dcmqrscp.cfg\nMaxPDUSize = 16384\nMaxAssociations = 16\n\nHostTable BEGIN\nHostTable END\n\nVendorTable BEGIN\nVendorTable END\n\nAETable BEGIN\nQR_SCP  {}  RW  (9, 1024mb)  ANY\nAETable END\n",
        abs_db.to_string_lossy()
    );
    std::fs::create_dir_all(&base).expect("create cfg dir");
    std::fs::write(&cfg_path, cfg).expect("write cfg");

    let mut dcmqr = tokio::process::Command::new("dcmqrscp");
    if verbose {
        dcmqr.arg("-d");
    }
    let dcmqr = dcmqr.arg("-c").arg(&cfg_path).arg(port.to_string());
    if !verbose {
        dcmqr
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let child = dcmqr.kill_on_drop(true).spawn().expect("spawn dcmqrscp");

    // Wait for port to be ready
    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    (child, port, base)
}

/// Helper to create a minimal DICOM dataset and store it via storescu
async fn store_test_dataset(
    port: u16,
    base: &PathBuf,
    patient_id: &str,
    verbose: bool,
) -> anyhow::Result<()> {
    let mkuid = |suf: &str| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        format!(
            "1.2.826.0.1.3680043.10.5432.{}.{}.{}",
            suf,
            now.as_secs(),
            now.subsec_nanos()
        )
    };

    let identifier = serde_json::json!({
        "00080016": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.7"] },
        "00080018": { "vr": "UI", "Value": [ mkuid("1") ] },
        "0020000D": { "vr": "UI", "Value": [ mkuid("2") ] },
        "0020000E": { "vr": "UI", "Value": [ mkuid("3") ] },
        "00080060": { "vr": "CS", "Value": [ "OT" ] },
        "00100020": { "vr": "LO", "Value": [patient_id] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "DOE^TEST"}] }
    });
    let obj = dicom_json_tool::json_value_to_identifier(&identifier)?;

    let dicom_path = base.join(format!("seed_{}.dcm", patient_id));
    dicom_json_tool::write_part10(&dicom_path, &obj)?;

    let mut st = tokio::process::Command::new("storescu");
    let st = st
        .arg("--aetitle")
        .arg("HARMONY_SCU")
        .arg("--call")
        .arg("QR_SCP")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg(&dicom_path);
    if !verbose {
        st.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let status = st.status().await?;
    if !status.success() {
        anyhow::bail!("storescu failed");
    }
    Ok(())
}

/// Test 1: dicom_scu backend with outgoing C-FIND
#[tokio::test]
async fn test_dicom_scu_backend_cfind() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let (mut qr_child, port, base) = spawn_dcmqrscp(verbose).await;

    // Store test data
    if store_test_dataset(port, &base, "SCU_TEST1", verbose)
        .await
        .is_err()
    {
        eprintln!("Failed to store test dataset; skipping test");
        let _ = qr_child.kill().await;
        return;
    }

    let toml = format!(
        r#"
        [proxy]
        id = "scu-backend-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8081

        [pipelines.scu_bridge]
        description = "HTTP -> DICOM SCU backend"
        networks = ["default"]
        endpoints = ["http_ep"]
        backends = ["scu_backend"]
        middleware = []

        [endpoints.http_ep]
        service = "http"
        [endpoints.http_ep.options]
        path_prefix = "/dicom"

        [backends.scu_backend]
        service = "dicom_scu"
        [backends.scu_backend.options]
        aet = "QR_SCP"
        host = "127.0.0.1"
        port = {port}
        local_aet = "HARMONY_SCU"

        [services.http]
        module = ""
        [services.dicom_scu]
        module = ""
    "#,
        port = port
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let body = serde_json::json!({
        "identifier": {
            "00100020": { "vr": "LO", "Value": ["SCU_TEST1"] }
        }
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicom/find")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json parse");
    assert_eq!(json.get("success").and_then(|v| v.as_bool()), Some(true));

    let _ = qr_child.kill().await;
}

/// Test 2: dicom_scp endpoint with incoming C-FIND
/// NOTE: This test requires the full service to be running with SCP listeners.
/// Currently ignored as it requires starting the DIMSE adapter's SCP listener.
#[tokio::test]
#[ignore]
async fn test_dicom_scp_endpoint_cfind() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    // Allocate a port for the Harmony SCP listener
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let scp_port = listener.local_addr().unwrap().port();
    drop(listener);

    let toml = format!(
        r#"
        [proxy]
        id = "scp-endpoint-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8082

        [pipelines.scp_listener]
        description = "DICOM SCP endpoint"
        networks = ["default"]
        endpoints = ["scp_ep"]
        backends = []
        middleware = []

        [endpoints.scp_ep]
        service = "dicom_scp"
        [endpoints.scp_ep.options]
        local_aet = "HARMONY_SCP"
        bind_addr = "127.0.0.1"
        port = {scp_port}
        enable_echo = true
        enable_find = true

        [services.dicom_scp]
        module = ""
    "#,
        scp_port = scp_port
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let _app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Spawn the Harmony router in a background task
    let _app_handle = tokio::spawn(async move {
        // This would normally be served via axum::Server, but for testing we just keep it alive
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });

    // Wait for SCP port to be ready
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Use findscu to query the Harmony SCP endpoint
    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let mut findscu = tokio::process::Command::new("findscu");
    findscu
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("HARMONY_SCP")
        .arg("-P")
        .arg("127.0.0.1")
        .arg(scp_port.to_string())
        .arg("-k")
        .arg("0010,0020=*");
    if !verbose {
        findscu
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }

    let status = findscu.status().await.expect("run findscu");
    // findscu returns 0 on success (even if no matches found)
    assert!(status.success(), "findscu should complete successfully");
}

/// Test 3: Configuration validation - dicom_scu cannot be used as endpoint
#[tokio::test]
async fn test_scu_cannot_be_endpoint() {
    let toml = r#"
        [proxy]
        id = "invalid-scu-endpoint"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8083

        [pipelines.invalid]
        description = "Invalid: SCU as endpoint"
        networks = ["default"]
        endpoints = ["scu_ep"]
        backends = []
        middleware = []

        [endpoints.scu_ep]
        service = "dicom_scu"
        [endpoints.scu_ep.options]
        aet = "REMOTE"
        host = "127.0.0.1"
        port = 4242
        local_aet = "HARMONY"

        [services.dicom_scu]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_err(),
        "dicom_scu used as endpoint should fail validation"
    );
}

/// Test 4: Configuration validation - dicom_scp cannot be used as backend
#[tokio::test]
async fn test_scp_cannot_be_backend() {
    let toml = r#"
        [proxy]
        id = "invalid-scp-backend"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8084

        [pipelines.invalid]
        description = "Invalid: SCP as backend"
        networks = ["default"]
        endpoints = ["http_ep"]
        backends = ["scp_backend"]
        middleware = []

        [endpoints.http_ep]
        service = "http"
        [endpoints.http_ep.options]
        path_prefix = "/dicom"

        [backends.scp_backend]
        service = "dicom_scp"
        [backends.scp_backend.options]
        local_aet = "HARMONY_SCP"
        port = 11112

        [services.http]
        module = ""
        [services.dicom_scp]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_err(),
        "dicom_scp used as backend should fail validation"
    );
}

/// Test 5: Backward compatibility - legacy "dicom" service maps to dicom_scu
#[tokio::test]
async fn test_legacy_dicom_service_backward_compat() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let (mut qr_child, port, base) = spawn_dcmqrscp(verbose).await;

    if store_test_dataset(port, &base, "LEGACY_TEST", verbose)
        .await
        .is_err()
    {
        eprintln!("Failed to store test dataset; skipping test");
        let _ = qr_child.kill().await;
        return;
    }

    let toml = format!(
        r#"
        [proxy]
        id = "legacy-dicom-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8085

        [pipelines.legacy]
        description = "Legacy dicom service"
        networks = ["default"]
        endpoints = ["http_ep"]
        backends = ["legacy_backend"]
        middleware = []

        [endpoints.http_ep]
        service = "http"
        [endpoints.http_ep.options]
        path_prefix = "/dicom"

        [backends.legacy_backend]
        service = "dicom"
        [backends.legacy_backend.options]
        aet = "QR_SCP"
        host = "127.0.0.1"
        port = {port}
        local_aet = "HARMONY_SCU"

        [services.http]
        module = ""
        [services.dicom]
        module = ""
    "#,
        port = port
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let body = serde_json::json!({
        "identifier": {
            "00100020": { "vr": "LO", "Value": ["LEGACY_TEST"] }
        }
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dicom/find")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json parse");
    assert_eq!(json.get("success").and_then(|v| v.as_bool()), Some(true));

    let _ = qr_child.kill().await;
}

/// Test 6: Pipeline integration - SCP endpoint receiving from external SCU
/// NOTE: This test requires the full service to be running with SCP listeners.
/// Currently ignored as it requires starting the DIMSE adapter's SCP listener.
#[tokio::test]
#[ignore]
async fn test_pipeline_scp_receives_external_find() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let scp_port = listener.local_addr().unwrap().port();
    drop(listener);

    let toml = format!(
        r#"
        [proxy]
        id = "pipeline-scp-test"
        log_level = "debug"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8086

        [pipelines.scp_pipeline]
        description = "SCP endpoint pipeline"
        networks = ["default"]
        endpoints = ["scp_ep"]
        backends = []
        middleware = []

        [endpoints.scp_ep]
        service = "dicom_scp"
        [endpoints.scp_ep.options]
        local_aet = "HARMONY_SCP"
        bind_addr = "127.0.0.1"
        port = {scp_port}
        enable_echo = true
        enable_find = true
        enable_get = true

        [services.dicom_scp]
        module = ""
    "#,
        scp_port = scp_port
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let _app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Give the SCP listener time to start
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Test C-ECHO first
    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let mut echoscu = tokio::process::Command::new("echoscu");
    echoscu
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("HARMONY_SCP")
        .arg("127.0.0.1")
        .arg(scp_port.to_string());
    if !verbose {
        echoscu
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }

    let status = echoscu.status().await.expect("run echoscu");
    assert!(status.success(), "C-ECHO should succeed");

    // Test C-FIND
    let mut findscu = tokio::process::Command::new("findscu");
    findscu
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("HARMONY_SCP")
        .arg("-P")
        .arg("127.0.0.1")
        .arg(scp_port.to_string())
        .arg("-k")
        .arg("0010,0020=*");
    if !verbose {
        findscu
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }

    let status = findscu.status().await.expect("run findscu");
    assert!(status.success(), "C-FIND should complete successfully");
}

/// Test 7: Full pipeline - HTTP -> SCU backend -> external PACS
#[tokio::test]
async fn test_full_pipeline_http_to_scu_to_pacs() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let (mut qr_child, qr_port, base) = spawn_dcmqrscp(verbose).await;

    if store_test_dataset(qr_port, &base, "FULL_PIPELINE", verbose)
        .await
        .is_err()
    {
        eprintln!("Failed to store test dataset; skipping test");
        let _ = qr_child.kill().await;
        return;
    }

    let toml = format!(
        r#"
        [proxy]
        id = "full-pipeline-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8087

        [pipelines.full]
        description = "Complete HTTP -> SCU -> PACS pipeline"
        networks = ["default"]
        endpoints = ["http_ep"]
        backends = ["scu_backend"]
        middleware = []

        [endpoints.http_ep]
        service = "http"
        [endpoints.http_ep.options]
        path_prefix = "/api"

        [backends.scu_backend]
        service = "dicom_scu"
        [backends.scu_backend.options]
        aet = "QR_SCP"
        host = "127.0.0.1"
        port = {qr_port}
        local_aet = "HARMONY_SCU"

        [services.http]
        module = ""
        [services.dicom_scu]
        module = ""
    "#,
        qr_port = qr_port
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let body = serde_json::json!({
        "identifier": {
            "00100020": { "vr": "LO", "Value": ["FULL_PIPELINE"] }
        }
    });
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/find")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json parse");
    assert_eq!(json.get("success").and_then(|v| v.as_bool()), Some(true));
    let matches = json
        .get("matches")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !matches.is_empty(),
        "Expected at least one C-FIND match in full pipeline"
    );

    let _ = qr_child.kill().await;
}
