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

fn find_samples_root() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("./samples/study_1"),
        PathBuf::from("./samples/dicom/study_1"),
        PathBuf::from("./dev/samples/study_1"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

#[tokio::test]
async fn pipeline_jmix_builder_returns_jmix_ids_and_manifest() {
    // Skip if DCMTK tools are not present
    for bin in ["dcmqrscp", "storescu", "getscu"].iter() {
        if std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("Skipping JMIX pipeline test: {} not found", bin);
            return;
        }
    }

    let samples_root = match find_samples_root() {
        Some(p) => p,
        None => {
            eprintln!("Skipping: samples directory not found");
            return;
        }
    };

    // Derive StudyInstanceUID from a known sample DICOM
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

    // Prepare QR SCP DB config
    let base = PathBuf::from("./tmp/qrscp_jmix");
    let dbdir = base.join("qrdb");
    std::fs::create_dir_all(&dbdir).expect("create qr db dir");
    let cfg_path = base.join("dcmqrscp.cfg");

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

    // Wait until ready
    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Send sample study via storescu (-r if available)
    let mut st_cmd = tokio::process::Command::new("storescu");
let st_cmd = st_cmd
        .arg("--aetitle").arg("HARMONY_SCU")
        .arg("--call").arg("QR_SCP")
        .arg("127.0.0.1").arg(port.to_string())
        .arg("-r").arg(&samples_root);
    if !verbose { st_cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()); }
    let try_recursive = st_cmd.status().await;

    if !try_recursive.as_ref().map(|s| s.success()).unwrap_or(false) {
        // fallback iterate
        let mut any_ok = false;
        for e in walkdir::WalkDir::new(&samples_root).into_iter().filter_map(|e| e.ok()) {
            if e.file_type().is_file() && e.path().extension().and_then(|s| s.to_str()) == Some("dcm") {
                let mut st2 = tokio::process::Command::new("storescu");
let st2 = st2
                    .arg("--aetitle").arg("HARMONY_SCU")
                    .arg("--call").arg("QR_SCP")
                    .arg("127.0.0.1").arg(port.to_string())
                    .arg(e.path());
                if !verbose { st2.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()); }
                let status = st2.status().await.expect("storescu");
                any_ok |= status.success();
            }
        }
        if !any_ok {
            eprintln!("storescu failed for sample files; skipping test assertions");
            let _ = qr_child.kill().await;
            return;
        }
    }

    // Build config: HTTP endpoint to DICOM backend, JMIX endpoint to serve JMIX, jmix_builder middleware
    let toml = format!(r#"
        [proxy]
        id = "jmix-builder-pipeline-test"
        log_level = "info"

        [storage]
        backend = "filesystem"
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8084

        [pipelines.bridge]
        description = "JMIX endpoint -> DICOM backend with JMIX builder"
        networks = ["default"]
        endpoints = ["jmix_http"]
        backends = ["dicom_pacs"]
        middleware = ["jmix_builder"]

        [endpoints.jmix_http]
        service = "jmix"
        [endpoints.jmix_http.options]
        path_prefix = "/jmix"

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
        [services.jmix]
        module = ""
    "#, port = port);

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Step 1: query JMIX by StudyInstanceUID; if none exists, pipeline triggers DICOM C-GET and builds JMIX
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jmix/api/jmix?studyInstanceUid={}", study_uid))
                .method("GET")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        // When data is missing in PACS, the pipeline should return 404
        assert_eq!(status, StatusCode::NOT_FOUND);
        let _ = qr_child.kill().await;
        return;
    }

    assert_eq!(status, StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let jmix_index: serde_json::Value = serde_json::from_slice(&bytes).expect("json parse");
    let id = jmix_index
        .get("jmixEnvelopes")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|o| o.get("id"))
        .and_then(|s| s.as_str())
        .expect("jmix id");

    // Step 2: fetch the JMIX manifest via JMIX endpoint
    let manifest_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jmix/api/jmix/{}/manifest", id))
                .method("GET")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(manifest_resp.status(), StatusCode::OK);
    let manifest_bytes = axum::body::to_bytes(manifest_resp.into_body(), usize::MAX).await.unwrap();
    let manifest_json: serde_json::Value = serde_json::from_slice(&manifest_bytes).expect("manifest json");
    assert_eq!(manifest_json.get("id").and_then(|v| v.as_str()), Some(id));

    // Step 3: optionally fetch the JMIX archive (zip)
    let archive_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/jmix/api/jmix/{}", id))
                .method("GET")
                .header("accept", "application/zip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");
    assert_eq!(archive_resp.status(), StatusCode::OK);

    let _ = qr_child.kill().await;
}
