use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

fn create_test_zip(
    source_dir: &PathBuf,
    zip_path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    use zip::write::FileOptions;

    let zip_file = std::fs::File::create(zip_path)?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    // Add manifest.json to zip
    let manifest_path = source_dir.join("manifest.json");
    if manifest_path.exists() {
        zip.start_file("manifest.json", options)?;
        let manifest_content = std::fs::read(&manifest_path)?;
        zip.write_all(&manifest_content)?;
    }

    // Add payload/metadata.json to zip
    let metadata_path = source_dir.join("payload").join("metadata.json");
    if metadata_path.exists() {
        zip.start_file("payload/metadata.json", options)?;
        let metadata_content = std::fs::read(&metadata_path)?;
        zip.write_all(&metadata_content)?;
    }

    zip.finish()?;
    Ok(())
}

fn ensure_jmix_envelope(id: &str) -> PathBuf {
    let root = PathBuf::from("./tmp/jmix-store");
    let pkg = root.join(id); // No .jmix extension - just the UUID
    let payload = pkg.join("payload");
    fs::create_dir_all(&payload).expect("mkdir jmix payload");
    let manifest = serde_json::json!({
        "id": id,
        "type": "envelope",
        "version": 1,
        "content": {"type": "directory", "path": "payload"}
    });
    fs::write(
        pkg.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .expect("write manifest");

    // Also write a minimal metadata.json for completeness
    let metadata = serde_json::json!({
        "id": id,
        "studies": {
            "study_uid": "1.2.3.4.5.test"
        }
    });
    fs::write(
        pkg.join("payload").join("metadata.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .expect("write metadata");

    // Create a minimal zip file for the package (as expected by JMIX middleware)
    let zip_path = pkg.join(format!("{}.zip", id));
    create_test_zip(&pkg, &zip_path).expect("create test zip");

    pkg
}

#[tokio::test]
async fn jmix_manifest_and_archive_skip_backends() {
    // Initialize storage backend before creating test data
    use harmony::storage::filesystem::FilesystemStorage;
    let storage = Arc::new(FilesystemStorage::new("./tmp").expect("Failed to create test storage"));
    harmony::globals::set_storage(storage);

    // Prepare a valid JMIX package on disk
    let id = Uuid::new_v4().to_string();
    let pkg_dir = ensure_jmix_envelope(&id);
    assert!(pkg_dir.exists());

    // Build config with an intentionally invalid DICOM backend (missing AET)
    // If backends are not skipped for JMIX-served routes, requests would fail with 502.
    let toml = format!(
        r#"
        [proxy]
        id = "jmix-skip-backends-test"
        log_level = "info"

        [storage]
        backend = "filesystem"
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8090

        [pipelines.bridge]
        description = "JMIX routes, invalid backend present"
        networks = ["default"]
        endpoints = ["jmix_http"]
        backends = ["dicom_bad"]
        middleware = ["jmix_builder"]

        [endpoints.jmix_http]
        service = "jmix"
        [endpoints.jmix_http.options]
        path_prefix = "/jmix"

        [backends.dicom_bad]
        service = "dicom"
        [backends.dicom_bad.options]
        host = "127.0.0.1"
        port = 104

        [middleware.jmix_builder]
        type = "jmix_builder"
        
        [services.http]
        module = ""
        [services.dicom]
        module = ""
        [services.jmix]
        module = ""
    "#
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // 1) Manifest route should be served by JMIX with 200 OK and include id
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
    let manifest_bytes = axum::body::to_bytes(manifest_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let manifest_json: serde_json::Value = serde_json::from_slice(&manifest_bytes).expect("json");
    assert_eq!(
        manifest_json.get("id").and_then(|v| v.as_str()),
        Some(id.as_str())
    );

    // 2) Archive route should also succeed with 200 OK and return a binary (zip by default)
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
    let archive_bytes = axum::body::to_bytes(archive_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(!archive_bytes.is_empty(), "expected non-empty archive body");
}
