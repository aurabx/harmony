use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn dicom_get_writes_samples_to_tmp() {
    // Skip if DCMTK tools are not present
    for bin in ["dcmqrscp", "storescu", "getscu"].iter() {
        if std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("Skipping sample C-GET test: {} not found", bin);
            return;
        }
    }

    // Locate sample files (prefer ./samples; fallback to ./dev/samples)
    let candidates = [
        PathBuf::from("./samples/study_1"),
        PathBuf::from("./samples/dicom/study_1"),
        PathBuf::from("./dev/samples/study_1"),
    ];
    let samples_root = candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("./dev/samples/study_1"));
    if !samples_root.exists() {
        eprintln!("Skipping: samples directory missing at {:?}", samples_root);
        return;
    }

    // Derive StudyInstanceUID from first sample file
    let first_sample = samples_root.join("series_1").join("CT.1.1.dcm");
    let obj = match dicom_object::open_file(&first_sample) {
        Ok(o) => o,
        Err(_) => {
            eprintln!("Skipping: failed to open first sample {:?}", first_sample);
            return;
        }
    };
    let study_uid_el = obj
        .element(dicom_dictionary_std::tags::STUDY_INSTANCE_UID)
        .ok();
    let study_uid: String = match study_uid_el
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(uid) => uid,
        None => {
            eprintln!("Skipping: missing StudyInstanceUID in {:?}", first_sample);
            return;
        }
    };

    // Pick a free port for QR SCP
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Prepare QR storage directory and config
    let base = PathBuf::from("./tmp/qrscp_samples");
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

    // Start dcmqrscp
    let mut qr_child = tokio::process::Command::new("dcmqrscp")
        .arg("-d")
        .arg("-c")
        .arg(&cfg_path)
        .arg(port.to_string())
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

    // Send all sample files via storescu (recursive)
    // Not all storescu builds support -r; if unavailable, fall back to iterating files.
    let try_recursive = tokio::process::Command::new("storescu")
        .arg("--aetitle")
        .arg("HARMONY_SCU")
        .arg("--call")
        .arg("QR_SCP")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg("-r")
        .arg(&samples_root)
        .status()
        .await;

    if !try_recursive.as_ref().map(|s| s.success()).unwrap_or(false) {
        // Fallback: iterate .dcm files
        let mut ok_any = false;
        for entry in walkdir::WalkDir::new(&samples_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|e| e.to_str()) == Some("dcm")
            {
                let status = tokio::process::Command::new("storescu")
                    .arg("--aetitle")
                    .arg("HARMONY_SCU")
                    .arg("--call")
                    .arg("QR_SCP")
                    .arg("127.0.0.1")
                    .arg(port.to_string())
                    .arg(entry.path())
                    .status()
                    .await
                    .expect("run storescu");
                ok_any |= status.success();
            }
        }
        if !ok_any {
            eprintln!("storescu failed for sample files; skipping assertions");
            let _ = qr_child.kill().await;
            return;
        }
    }

    // Build Harmony config with DICOM backend pointing to QR_SCP
    let toml = format!(
        r#"
        [proxy]
        id = "dicom-get-samples-test"
        log_level = "info"
        
        [storage]
        backend = "filesystem"
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8082

        [pipelines.bridge]
        description = "HTTP -> DICOM backend bridge (samples)"
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

    // POST /dicom/get with StudyInstanceUID from sample
    let body = serde_json::json!({
        "identifier": {
            "0020000D": { "vr": "UI", "Value": [ study_uid ] }
        }
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dicom/get")
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
    assert_eq!(json.get("operation").and_then(|v| v.as_str()), Some("get"));
    assert_eq!(json.get("success").and_then(|v| v.as_bool()), Some(true));

    // Verify folder info and that files exist in the folder
    let folder_path = json
        .get("folder_path")
        .and_then(|v| v.as_str())
        .expect("missing folder_path");
    let out_dir = Path::new(folder_path);
    assert!(
        out_dir.exists(),
        "folder_path does not exist: {}",
        folder_path
    );

    let file_count = json
        .get("file_count")
        .and_then(|v| v.as_u64())
        .expect("missing file_count");
    assert!(file_count > 0, "file_count should be > 0");

    // Count .dcm files in the output directory and ensure we got at least as many as one series
    let mut saved_dcms = 0usize;
    for e in std::fs::read_dir(out_dir).expect("read out dir").flatten() {
        if e.path().extension().and_then(|s| s.to_str()) == Some("dcm") {
            saved_dcms += 1;
        }
    }
    assert!(
        saved_dcms >= 1,
        "expected at least 1 saved DICOM file in {}",
        folder_path
    );

    let _ = qr_child.kill().await;
}
