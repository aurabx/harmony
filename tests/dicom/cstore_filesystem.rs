use harmony::config::config::{Config, ConfigError};
use std::path::PathBuf;
use uuid::Uuid;

mod helpers;
use helpers::ScpTestHarness;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

fn dcmtk_available() -> bool {
    std::process::Command::new("storescu")
        .arg("--version")
        .output()
        .is_ok()
}

async fn create_dummy_dicom_file(path: &PathBuf) -> anyhow::Result<()> {
    let identifier = serde_json::json!({
        "00080016": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.7"] }, // Secondary Capture Image Storage
        "00080018": { "vr": "UI", "Value": [Uuid::new_v4().urn().to_string()] }, // SOP Instance UID
        "0020000D": { "vr": "UI", "Value": [Uuid::new_v4().urn().to_string()] }, // Study Instance UID
        "0020000E": { "vr": "UI", "Value": [Uuid::new_v4().urn().to_string()] }, // Series Instance UID
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "TEST^FILESYSTEM"}] },
        "00100020": { "vr": "LO", "Value": ["TEST_ID_FS"] }
    });
    let obj = dicom_json_tool::json_value_to_identifier(&identifier)?;
    dicom_json_tool::write_part10(path, &obj)?;
    Ok(())
}

#[tokio::test]
async fn test_cstore_filesystem_backend() {
    if !dcmtk_available() {
        eprintln!("Skipping test: storescu (DCMTK) not available");
        return;
    }

    // Setup temporary directories
    let storage_root = tempfile::tempdir().expect("create storage root");
    let input_dir = tempfile::tempdir().expect("create input dir");
    let storage_path = storage_root.path().to_string_lossy().to_string();

    // Allocate a random port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let scp_port = listener.local_addr().unwrap().port();
    drop(listener);

    let toml = format!(
        r#"
        [proxy]
        id = "cstore-fs-test"
        log_level = "info"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "{storage_path}"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 0 # We don't care about HTTP port for this test

        [pipelines.store_pipeline]
        description = "C-STORE to filesystem"
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
        enable_store = true

        [services.dicom_scp]
        module = ""
    "#
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");

    // Start Harmony SCP
    let mut harness = ScpTestHarness::new(cfg, "default");
    harness.start().await.expect("Failed to start test harness");

    // Create a dummy DICOM file to send
    let dcm_file = input_dir.path().join("test.dcm");
    create_dummy_dicom_file(&dcm_file)
        .await
        .expect("create dummy dicom");

    // Send C-STORE using storescu
    let status = tokio::process::Command::new("storescu")
        .arg("-v")
        .arg("-aet")
        .arg("TEST_SCU")
        .arg("-aec")
        .arg("HARMONY_SCP")
        .arg("127.0.0.1")
        .arg(scp_port.to_string())
        .arg(&dcm_file)
        .status()
        .await
        .expect("run storescu");

    assert!(status.success(), "storescu should succeed");

    // Verify file exists in storage
    // The directory structure should be: {storage_root}/dimse/{uuid}.dcm
    let dimse_dir = storage_root.path().join("dimse");
    assert!(dimse_dir.exists(), "dimse storage directory should exist");

    let mut found = false;
    let mut entries = tokio::fs::read_dir(&dimse_dir).await.expect("read dimse dir");
    while let Some(entry) = entries.next_entry().await.expect("next entry") {
        if entry
            .path()
            .extension()
            .map(|e| e == "dcm")
            .unwrap_or(false)
        {
            found = true;
            // Optional: Read file and verify SOP Instance UID matches
            break;
        }
    }

    assert!(found, "Should find at least one .dcm file in storage");

    harness.shutdown().await.expect("shutdown harness");
}
