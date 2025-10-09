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

#[tokio::test]
async fn dicom_find_with_dcmqrscp() {
    // Skip if DCMTK tools are not present
    for bin in ["dcmqrscp", "storescu", "findscu", "dcmqridx"].iter() {
        if std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("Skipping dcmqrscp test: {} not found", bin);
            return;
        }
    }

    // Pick a free port for QR SCP
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Prepare QR storage directory and config
    let base = PathBuf::from("./tmp/qrscp");
    let dbdir = base.join("qrdb");
    std::fs::create_dir_all(&dbdir).expect("create qr db dir");
    let cfg_path = base.join("dcmqrscp.cfg");

    // Use absolute path for database directory
    let abs_db = match std::fs::canonicalize(&dbdir) {
        Ok(p) => p,
        Err(_) => std::env::current_dir().unwrap().join(&dbdir),
    };

    let cfg = format!(
        "# Minimal dcmqrscp.cfg\nMaxPDUSize = 16384\nMaxAssociations = 16\n\nHostTable BEGIN\nHostTable END\n\nVendorTable BEGIN\nVendorTable END\n\nAETable BEGIN\nQR_SCP  {db}  RW  (9, 1024mb)  ANY\nAETable END\n",
        db = abs_db.to_string_lossy()
    );
    std::fs::create_dir_all(&base).expect("create cfg dir");
    std::fs::write(&cfg_path, cfg).expect("write cfg");

    // Start dcmqrscp (quiet by default; enable verbose with HARMONY_TEST_VERBOSE_DCMTK=1)
    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let mut dcmqr = tokio::process::Command::new("dcmqrscp");
    if verbose { dcmqr.arg("-d"); }
let dcmqr = dcmqr
        .arg("-c")
        .arg(&cfg_path)
        .arg(port.to_string());
    if !verbose { dcmqr.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()); }
    let mut qr_child = dcmqr
        .kill_on_drop(true)
        .spawn()
        .expect("spawn dcmqrscp");

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

    // Build a minimal dataset with PatientID=TEST123 and write to file
    // Build a minimal identifier with required C-STORE fields
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
        // SOP Class: Secondary Capture Image Storage
        "00080016": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.7"] },
        // SOP Instance UID
        "00080018": { "vr": "UI", "Value": [ mkuid("1") ] },
        // Study/Series Instance UIDs
        "0020000D": { "vr": "UI", "Value": [ mkuid("2") ] },
        "0020000E": { "vr": "UI", "Value": [ mkuid("3") ] },
        // Modality
        "00080060": { "vr": "CS", "Value": [ "OT" ] },
        // Patient ID / Name
        "00100020": { "vr": "LO", "Value": ["TEST123"] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "DOE^FIND"}] }
    });
    let obj = dicom_json_tool::json_value_to_identifier(&identifier).expect("json->obj");

    let dicom_path = base.join("seed.dcm");
    dicom_json_tool::write_part10(&dicom_path, &obj).expect("write seed");

    // Send the dataset to QR via storescu
    let mut st = tokio::process::Command::new("storescu");
let st = st
        .arg("--aetitle")
        .arg("HARMONY_SCU")
        .arg("--call")
        .arg("QR_SCP")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg(&dicom_path);
    if !verbose { st.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()); }
    let status = st
        .status()
        .await
        .expect("run storescu");
    if !status.success() {
        eprintln!("storescu failed; skipping assertions");
        let _ = qr_child.kill().await;
        return;
    }

    // Build Harmony config with DICOM backend pointing to QR_SCP
    let toml = format!(
        r#"
        [proxy]
        id = "dicom-find-test"
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

    // POST /dicom/find with identifier JSON
    let body = serde_json::json!({
        "identifier": {
            "00100020": { "vr": "LO", "Value": ["TEST123"] }
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
    assert_eq!(json.get("operation").and_then(|v| v.as_str()), Some("find"));
    assert_eq!(json.get("success").and_then(|v| v.as_bool()), Some(true));
    let matches = json
        .get("matches")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // We expect at least one match if everything succeeded
    assert!(
        !matches.is_empty(),
        "Expected at least one C-FIND match, got 0"
    );

    let _ = qr_child.kill().await;
}
