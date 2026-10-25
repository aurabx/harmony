//! A `storage` backend with `content = "dicom_part10"` keeps each instance a
//! `dicom_scp` endpoint receives, after the pipeline's policies admit it.
//!
//! Two pipelines share one config. Each has its own SCP port and archive
//! folder. Both allow 127.0.0.0/8, and the second also denies it, so only the
//! deny rule separates them.

use harmony::config::config::Config;
use std::path::{Path, PathBuf};
use uuid::Uuid;

mod helpers;
use helpers::ScpTestHarness;

fn dcmtk_available() -> bool {
    std::process::Command::new("storescu")
        .arg("--version")
        .output()
        .is_ok()
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    listener.local_addr().unwrap().port()
}

/// A UID under the 2.25 root, which takes a UUID as a decimal integer.
fn new_uid() -> String {
    format!("2.25.{}", Uuid::new_v4().as_u128())
}

struct TestInstance {
    path: PathBuf,
    study_uid: String,
    sop_uid: String,
}

fn write_instance(dir: &Path) -> TestInstance {
    let study_uid = new_uid();
    let sop_uid = new_uid();
    let identifier = serde_json::json!({
        "00080016": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.7"] },
        "00080018": { "vr": "UI", "Value": [sop_uid] },
        "0020000D": { "vr": "UI", "Value": [study_uid] },
        "0020000E": { "vr": "UI", "Value": [new_uid()] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "TEST^ARCHIVE"}] },
        "00100020": { "vr": "LO", "Value": ["TEST_ID_ARCHIVE"] }
    });
    let obj = dicom_json_tool::json_value_to_identifier(&identifier).expect("identifier");
    let path = dir.join(format!("{}.dcm", sop_uid));
    dicom_json_tool::write_part10(&path, &obj).expect("write instance");
    TestInstance {
        path,
        study_uid,
        sop_uid,
    }
}

async fn storescu(port: u16, file: &Path) -> std::process::ExitStatus {
    tokio::process::Command::new("storescu")
        .args(["-aet", "TEST_SCU", "-aec", "HARMONY_SCP", "127.0.0.1"])
        .arg(port.to_string())
        .arg(file)
        .status()
        .await
        .expect("run storescu")
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files.extend(files_under(&path));
        } else {
            files.push(path);
        }
    }
    files
}

fn config(
    adapter_storage: &Path,
    admit_root: &Path,
    admit_port: u16,
    deny_root: &Path,
    deny_port: u16,
) -> Config {
    let toml = format!(
        r#"
        [proxy]
        id = "cstore-storage-backend-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "{adapter_storage}"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 0

        [pipelines.admitted]
        networks = ["default"]
        endpoints = ["admit_scp"]
        middleware = ["admit_policy"]
        backends = ["admit_archive"]

        [pipelines.denied]
        networks = ["default"]
        endpoints = ["deny_scp"]
        middleware = ["deny_policy"]
        backends = ["deny_archive"]

        [endpoints.admit_scp]
        service = "dicom_scp"
        [endpoints.admit_scp.options]
        local_aet = "HARMONY_SCP"
        bind_addr = "127.0.0.1"
        port = {admit_port}
        enable_store = true

        [endpoints.deny_scp]
        service = "dicom_scp"
        [endpoints.deny_scp.options]
        local_aet = "HARMONY_SCP"
        bind_addr = "127.0.0.1"
        port = {deny_port}
        enable_store = true

        [backends.admit_archive]
        service = "storage"
        [backends.admit_archive.options]
        root = "{admit_root}"
        content = "dicom_part10"
        write_pattern = "{{StudyInstanceUID}}/{{SOPInstanceUID}}.dcm"

        [backends.deny_archive]
        service = "storage"
        [backends.deny_archive.options]
        root = "{deny_root}"
        content = "dicom_part10"
        write_pattern = "{{StudyInstanceUID}}/{{SOPInstanceUID}}.dcm"

        [middleware.admit_policy]
        type = "policies"
        [middleware.admit_policy.options]
        policies = ["admit"]

        [middleware.deny_policy]
        type = "policies"
        [middleware.deny_policy.options]
        policies = ["deny"]

        [policies.admit]
        id = "admit"
        name = "Admit loopback"
        enabled = true
        rules = ["allow_loopback"]

        [policies.deny]
        id = "deny"
        name = "Deny loopback"
        enabled = true
        rules = ["allow_loopback", "deny_loopback"]

        [rules.allow_loopback]
        id = "allow_loopback"
        name = "Allow loopback"
        type = "ip_allow"
        weight = 100
        enabled = true
        [rules.allow_loopback.options]
        ip_addresses = ["127.0.0.0/8"]

        [rules.deny_loopback]
        id = "deny_loopback"
        name = "Deny loopback"
        type = "ip_deny"
        weight = 90
        enabled = true
        [rules.deny_loopback.options]
        ip_addresses = ["127.0.0.0/8"]

        [middleware_types.policies]
        module = ""

        [services.dicom_scp]
        module = ""

        [services.storage]
        module = ""
        "#,
        adapter_storage = adapter_storage.display(),
        admit_root = admit_root.display(),
        deny_root = deny_root.display(),
    );
    let config: Config = toml::from_str(&toml).expect("TOML parse error");
    config.validate().expect("valid config");
    config
}

#[tokio::test]
async fn test_cstore_is_archived_only_when_policies_admit_it() {
    if !dcmtk_available() {
        eprintln!("Skipping test: storescu (DCMTK) not available");
        return;
    }

    let adapter_storage = tempfile::tempdir().expect("adapter storage");
    let admit_root = tempfile::tempdir().expect("admit archive");
    let deny_root = tempfile::tempdir().expect("deny archive");
    let input_dir = tempfile::tempdir().expect("input dir");
    let admit_port = free_port();
    let deny_port = free_port();

    let cfg = config(
        adapter_storage.path(),
        admit_root.path(),
        admit_port,
        deny_root.path(),
        deny_port,
    );
    let mut harness = ScpTestHarness::new(cfg, "default");
    harness.start().await.expect("start test harness");

    let admitted = write_instance(input_dir.path());
    let status = storescu(admit_port, &admitted.path).await;
    assert!(
        status.success(),
        "storescu to the admitted pipeline should succeed"
    );

    let archived = admit_root
        .path()
        .join(&admitted.study_uid)
        .join(format!("{}.dcm", admitted.sop_uid));
    let stored = dicom_object::open_file(&archived)
        .unwrap_or_else(|e| panic!("archived instance at {}: {}", archived.display(), e));
    assert_eq!(
        stored
            .meta()
            .media_storage_sop_instance_uid()
            .trim_end_matches('\0'),
        admitted.sop_uid
    );
    assert_eq!(files_under(admit_root.path()), vec![archived.clone()]);

    let denied = write_instance(input_dir.path());
    let status = storescu(deny_port, &denied.path).await;
    assert!(
        !status.success(),
        "the SCP must report a failure status for a denied C-STORE"
    );
    assert!(
        files_under(deny_root.path()).is_empty(),
        "a denied C-STORE must not be archived: {:?}",
        files_under(deny_root.path())
    );

    harness.shutdown().await.expect("shutdown harness");
}
