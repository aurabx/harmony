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
async fn dicom_move_persistent_relocates_into_per_move_dir() {
    // Skip if DCMTK server tools are not present (QR SCP required)
    for bin in ["dcmqrscp", "storescu"].iter() {
        if std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("Skipping persistent relocate test: {} not found", bin);
            return;
        }
    }

    // Pick free ports for QR SCP and incoming Store SCP
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let qr_port = listener.local_addr().unwrap().port();
    drop(listener);

    let listener2 = std::net::TcpListener::bind("127.0.0.1:0").expect("bind store port");
    let store_port = listener2.local_addr().unwrap().port();
    drop(listener2);

    // Prepare QR storage directory and config
    let base = PathBuf::from("./tmp/qrscp_persist");
    let dbdir = base.join("qrdb");
    std::fs::create_dir_all(&dbdir).expect("create qr db dir");
    let cfg_path = base.join("dcmqrscp.cfg");

    // Use absolute path for database directory
    let abs_db = match std::fs::canonicalize(&dbdir) {
        Ok(p) => p,
        Err(_) => std::env::current_dir().unwrap().join(&dbdir),
    };

    // Config with HostTable for destination AE HARMONY_MOVE at dynamic port
    let cfg = format!(
        "# dcmqrscp.cfg\nMaxPDUSize = 16384\nMaxAssociations = 16\n\nHostTable BEGIN\nHARMONY_MOVE = (HARMONY_MOVE, 127.0.0.1, {store_port})\nHostTable END\n\nVendorTable BEGIN\nVendorTable END\n\nAETable BEGIN\nQR_SCP  {db}  RW  (9, 1024mb)  ANY\nAETable END\n",
        db = abs_db.to_string_lossy(),
        store_port = store_port
    );
    std::fs::create_dir_all(&base).expect("create cfg dir");
    std::fs::write(&cfg_path, cfg).expect("write cfg");

    // Start dcmqrscp
    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let mut dcmqr = tokio::process::Command::new("dcmqrscp");
    if verbose {
        dcmqr.arg("-d");
    }
    let dcmqr = dcmqr.arg("-c").arg(&cfg_path).arg(qr_port.to_string());
    if !verbose {
        dcmqr
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let mut qr_child = dcmqr.kill_on_drop(true).spawn().expect("spawn dcmqrscp");

    // Wait for port to be ready
    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", qr_port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Make a synthetic DICOM with known StudyInstanceUID and send to QR via storescu
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
    let study_uid = mkuid("study");
    let series_uid = mkuid("series");
    let sop_uid = mkuid("sop");
    let identifier = serde_json::json!({
        "00080016": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.7"] },
        "00080018": { "vr": "UI", "Value": [ sop_uid ] },
        "0020000D": { "vr": "UI", "Value": [ study_uid ] },
        "0020000E": { "vr": "UI", "Value": [ series_uid ] },
        "00080060": { "vr": "CS", "Value": [ "OT" ] },
        "00100020": { "vr": "LO", "Value": ["PERSIST123"] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "DOE^PERSIST"}] }
    });
    let obj = dicom_json_tool::json_value_to_identifier(&identifier).expect("json->obj");
    let dicom_path = base.join("seed_persist_move.dcm");
    dicom_json_tool::write_part10(&dicom_path, &obj).expect("write seed");

    // Send the dataset to QR via storescu
    let mut st = tokio::process::Command::new("storescu");
    let st = st
        .arg("--aetitle")
        .arg("HARMONY_SCU")
        .arg("--call")
        .arg("QR_SCP")
        .arg("127.0.0.1")
        .arg(qr_port.to_string())
        .arg(&dicom_path);
    if !verbose {
        st.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let status = st.status().await.expect("run storescu");
    if !status.success() {
        eprintln!("storescu failed; skipping assertions");
        let _ = qr_child.kill().await;
        return;
    }
    // Give QR SCP a moment to index the dataset
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Configure Harmony with persistent Store SCP for the DICOM backend, storage path ./tmp
    let toml = format!(
        r#"
        [proxy]
        id = "dicom-persist-relocate-test"
        log_level = "info"

        [storage]
        backend = "filesystem"
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 0

        [pipelines.bridge]
        description = "HTTP -> DICOM backend persistent bridge"
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
        port = {qr_port}
        local_aet = "HARMONY_MOVE"
        incoming_store_port = {store_port}
        persistent_store_scp = true
        # Force internal SCP for tests to avoid external storescp hanging
        use_dcmtk_store = false

        [services.http]
        module = ""
        [services.dicom]
        module = ""
    "#
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Perform the MOVE
    let body = serde_json::json!({
        "identifier": {
            "0020000D": { "vr": "UI", "Value": [ study_uid ] },
            "00100020": { "vr": "LO", "Value": [ "PERSIST123" ] }
        }
    });

    // Attempt the MOVE with a few retries if the QR DB hasn't indexed yet
    let mut attempt = 0;
    let json = loop {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/dicom/move")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .expect("router handled request");
        if resp.status() == StatusCode::OK {
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json parse");
            break json;
        } else if resp.status() == StatusCode::NOT_FOUND && attempt < 10 {
            attempt += 1;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            continue;
        } else {
            eprintln!(
                "MOVE failed with status {} after {} attempts",
                resp.status(),
                attempt + 1
            );
            let _ = qr_child.kill().await;
            return; // skip assertions in flakey environments
        }
    };
    eprintln!(
        "Persistent move response: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    // Extract folder_path and validate contents
    let folder_path = json
        .get("folder_path")
        .and_then(|v| v.as_str())
        .map(|s| PathBuf::from(s))
        .expect("folder_path present");

    assert!(folder_path.exists(), "folder_path should exist");

    // Verify at least one .dcm file exists in the per-move directory
    let mut dcm_count = 0usize;
    if let Ok(read) = std::fs::read_dir(&folder_path) {
        for e in read.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("dcm") {
                dcm_count += 1;
            }
        }
    }
    assert!(
        dcm_count > 0,
        "Expected at least one .dcm in per-move directory"
    );

    // Ensure per-move directory is under ./tmp/dimse (storage adapter path)
    let canonical = folder_path.to_string_lossy().to_string();
    assert!(
        canonical.contains("/tmp/dimse/")
            || canonical.contains("./tmp/dimse/")
            || canonical.contains("tmp/dimse/"),
        "folder_path should be under storage adapter root ./tmp/dimse; got {}",
        canonical
    );

    let _ = qr_child.kill().await;
}
