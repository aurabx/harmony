use harmony::config::config::{Config, ConfigError};
use axum::http::{Request, StatusCode};
use axum::body::Body;
use tower::ServiceExt;
use std::sync::Arc;
use std::path::PathBuf;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn dicom_move_with_dcmqrscp() {
    // Skip if DCMTK tools are not present
    for bin in ["dcmqrscp", "storescu", "movescu"].iter() {
        if std::process::Command::new(bin).arg("--version").output().is_err() {
            eprintln!("Skipping dcmqrscp C-MOVE test: {} not found", bin);
            return;
        }
    }

    // Helper: recursively walk a directory and collect .dcm files (up to a limit)
    fn collect_dcm_files(root: &std::path::Path, max_files: usize) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>, max_files: usize) {
            if files.len() >= max_files { return; }
            if let Ok(read) = std::fs::read_dir(dir) {
                for entry in read.flatten() {
                    if files.len() >= max_files { break; }
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path, files, max_files);
                    } else if path.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("dcm")).unwrap_or(false) {
                        files.push(path);
                        if files.len() >= max_files { break; }
                    }
                }
            }
        }
        walk(root, &mut files, max_files);
        files
    }

    // Helper: send a single DICOM file to QR_SCP via storescu
    async fn send_via_storescu(file: &std::path::Path, port: u16) -> bool {
        match tokio::process::Command::new("storescu")
            .arg("--aetitle").arg("HARMONY_SCU")
            .arg("--call").arg("QR_SCP")
            .arg("127.0.0.1").arg(port.to_string())
            .arg(file)
            .status().await {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }

    // Optional diagnostic: run a C-FIND at STUDY level prior to MOVE
    async fn diagnostic_find_study(port: u16, study_uid: &str, patient_id: &str) {
        if std::env::var("HARMONY_TEST_DEBUG").ok().as_deref() != Some("1") {
            return;
        }
        if std::process::Command::new("findscu").arg("--version").output().is_err() {
            eprintln!("[diag] findscu not available, skipping pre-MOVE C-FIND");
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_else(|_| std::time::Duration::from_secs(0));
        let out_dir = std::path::PathBuf::from(format!("./tmp/dcmtk_find_diag_{}_{}", now.as_secs(), now.subsec_nanos()));
        let _ = std::fs::create_dir_all(&out_dir);
        let args = vec![
            "-S".to_string(),
            "-aet".to_string(), "HARMONY_MOVE".to_string(),
            "-aec".to_string(), "QR_SCP".to_string(),
            "-k".to_string(), "0008,0052=STUDY".to_string(),
            "-k".to_string(), format!("0020,000D={}", study_uid),
            "-k".to_string(), format!("0010,0020={}", patient_id),
            "-X".to_string(),
            "-od".to_string(), out_dir.to_string_lossy().to_string(),
            "127.0.0.1".to_string(), port.to_string(),
        ];
        eprintln!("[diag] Running findscu with args: {:?}", args);
        match tokio::process::Command::new("findscu").args(&args).output().await {
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let produced = std::fs::read_dir(&out_dir).ok()
                    .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("dcm")).count())
                    .unwrap_or(0);
                eprintln!("[diag] findscu status={:?}, out_dir dcm files={}, stdout=\n{}\n----\nstderr=\n{}", out.status.code(), produced, stdout, stderr);
            }
            Err(e) => eprintln!("[diag] Failed to spawn findscu: {}", e),
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

    // Official-format config: Host/Vendor tables present (empty), AETable uses RW and ANY peer
    let cfg = format!(
        "# Minimal dcmqrscp.cfg\nMaxPDUSize = 16384\nMaxAssociations = 16\n\nHostTable BEGIN\nHARMONY_SCU = (HARMONY_SCU, 127.0.0.1, 11123)\nHARMONY_MOVE = (HARMONY_MOVE, 127.0.0.1, 11124)\nHostTable END\n\nVendorTable BEGIN\nVendorTable END\n\nAETable BEGIN\nQR_SCP  {db}  RW  (9, 1024mb)  ANY\nAETable END\n",
        db = abs_db.to_string_lossy()
    );
    std::fs::create_dir_all(&base).expect("create cfg dir");
    std::fs::write(&cfg_path, cfg).expect("write cfg");

    // Start dcmqrscp
    let mut qr_child = tokio::process::Command::new("dcmqrscp")
        .arg("-d")
        .arg("-c").arg(&cfg_path)
        .arg(port.to_string())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn dcmqrscp");

    // Wait for port to be ready
    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() { break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Build a minimal identifier and Part 10 file with known StudyInstanceUID
    let mkuid = |suf: &str| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        format!("1.2.826.0.1.3680043.10.5432.{}.{}.{}", suf, now.as_secs(), now.subsec_nanos())
    };
    let study_uid = mkuid("study");
    let series_uid = mkuid("series");
    let sop_uid = mkuid("sop");
    let identifier = serde_json::json!({
        // SOP Class: Secondary Capture Image Storage
        "00080016": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.7"] },
        // SOP Instance UID
        "00080018": { "vr": "UI", "Value": [ sop_uid ] },
        // Study/Series Instance UIDs
        "0020000D": { "vr": "UI", "Value": [ study_uid ] },
        "0020000E": { "vr": "UI", "Value": [ series_uid ] },
        // Modality
        "00080060": { "vr": "CS", "Value": [ "OT" ] },
        // Patient ID / Name
        "00100020": { "vr": "LO", "Value": ["MOVE123"] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "DOE^MOVE"}] }
    });
    let obj = dicom_json_tool::json_value_to_identifier(&identifier).expect("json->obj");
    let dicom_path = base.join("seed_move.dcm");
    dicom_json_tool::write_part10(&dicom_path, &obj).expect("write seed");

    // Send the dataset to QR via storescu
    let status = tokio::process::Command::new("storescu")
        .arg("--aetitle").arg("HARMONY_SCU")
        .arg("--call").arg("QR_SCP")
        .arg("127.0.0.1").arg(port.to_string())
        .arg(&dicom_path)
        .status().await.expect("run storescu");
    if !status.success() {
        eprintln!("storescu failed; skipping assertions");
        let _ = qr_child.kill().await;
        return;
    }

    // Optionally preload dev/samples into QR SCP (up to 20 DICOMs)
    let samples_root = std::path::Path::new("dev/samples");
    if samples_root.exists() {
        let files = collect_dcm_files(samples_root, 20);
        if !files.is_empty() {
            eprintln!("Preloading {} sample DICOMs from dev/samples", files.len());
            for f in files {
                let ok = send_via_storescu(&f, port).await;
                if !ok {
                    eprintln!("Warning: storescu failed for {:?}", f);
                }
            }
        } else {
            eprintln!("No .dcm files found under dev/samples (skipping preload)");
        }
    }

    // Build Harmony config with DICOM backend pointing to QR_SCP
    let toml = format!(r#"
        [proxy]
        id = "dicom-move-test"
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
        local_aet = "HARMONY_MOVE"
        incoming_store_port = 11124

        [services.http]
        module = ""
        [services.dicom]
        module = ""
    "#, port=port);

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Optional pre-MOVE diagnostic C-FIND at STUDY level (debug only)
    diagnostic_find_study(port, &study_uid, "MOVE123").await;

    // POST /dicom/move with StudyInstanceUID
    let body = serde_json::json!({
        "identifier": {
            "0020000D": { "vr": "UI", "Value": [ study_uid ] },
            "00100020": { "vr": "LO", "Value": [ "MOVE123" ] }
        }
    });
    let response = app.clone()
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

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json parse");
    // Print full response including debug before assertions for diagnostics
    eprintln!("C-MOVE response: {}", serde_json::to_string_pretty(&json).unwrap());
    assert_eq!(json.get("operation").and_then(|v| v.as_str()), Some("move"));
    assert_eq!(json.get("success").and_then(|v| v.as_bool()), Some(true));
    let instances = json.get("instances").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    assert!(instances.len() >= 1, "Expected at least one C-MOVE instance, got 0");

    let _ = qr_child.kill().await;
}
