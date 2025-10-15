use harmony::config::config::{Config, ConfigError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::OnceCell;

// Shared test state to avoid spinning up backend/app multiple times
static TEST_STATE: OnceCell<TestContext> = OnceCell::const_new();

pub struct TestContext {
    pub app: axum::Router<()>,
    pub uids: TestUids,
}

/// UIDs and identifiers extracted from sample DICOM files at runtime
#[allow(dead_code)]
pub struct TestUids {
    pub study_uid: String,
    pub series_1_uid: String,
    pub series_2_uid: String,
    pub series_3_uid: String,
    pub instance_1_1_uid: String,
    pub instance_2_1_uid: String,
    pub instance_3_1_uid: String,
    pub patient_id: String,
}

/// Extract UIDs and identifiers from sample DICOM files using the dicom-object crate
fn extract_uids_from_samples() -> Option<TestUids> {
    let candidates = [
        PathBuf::from("./samples/study_1"),
        PathBuf::from("./dev/samples/study_1"),
    ];
    let samples_root = candidates.into_iter().find(|p| p.exists())?;

    // Extract from first file in each series
    let extract_uid = |path: &Path, tag_name: &str| -> Option<String> {
        if !path.exists() {
            return None;
        }
        let obj = dicom_object::open_file(path).ok()?;
        let tag_id = match tag_name {
            "study" => (0x0020, 0x000D),
            "series" => (0x0020, 0x000E),
            "sop" => (0x0008, 0x0018),
            "patient" => (0x0010, 0x0020),
            _ => return None,
        };
        obj.element(dicom_core::header::Tag(tag_id.0, tag_id.1))
            .ok()?
            .to_str()
            .ok()
            .map(|s| s.to_string())
    };

    // Extract Study UID (same for all files)
    let study_uid = extract_uid(&samples_root.join("series_1/CT.1.1.dcm"), "study")?;

    // Extract Series UIDs
    let series_1_uid = extract_uid(&samples_root.join("series_1/CT.1.1.dcm"), "series")?;
    let series_2_uid = extract_uid(&samples_root.join("series_2/CT.2.1.dcm"), "series")?;
    let series_3_uid = extract_uid(&samples_root.join("series_3/CT.3.1.dcm"), "series")?;

    // Extract SOP Instance UIDs
    let instance_1_1_uid = extract_uid(&samples_root.join("series_1/CT.1.1.dcm"), "sop")?;
    let instance_2_1_uid = extract_uid(&samples_root.join("series_2/CT.2.1.dcm"), "sop")?;
    let instance_3_1_uid = extract_uid(&samples_root.join("series_3/CT.3.1.dcm"), "sop")?;

    // Extract Patient ID
    let patient_id = extract_uid(&samples_root.join("series_1/CT.1.1.dcm"), "patient")?;

    Some(TestUids {
        study_uid,
        series_1_uid,
        series_2_uid,
        series_3_uid,
        instance_1_1_uid,
        instance_2_1_uid,
        instance_3_1_uid,
        patient_id,
    })
}

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

async fn build_router_with_config(toml: &str) -> Result<axum::Router<()>, ConfigError> {
    let _ = std::fs::create_dir_all("./tmp");
    let c = load_config_from_str(toml)?;
    let router = harmony::router::build_network_router(Arc::new(c), "default").await;
    Ok(router)
}

/// Initialize shared test context (backend and app) once for all tests
pub async fn get_test_context() -> &'static TestContext {
    TEST_STATE
        .get_or_init(|| async { setup_dicomweb_test().await })
        .await
}

async fn setup_dicomweb_test() -> TestContext {
    // Extract UIDs from samples for validation (even though we use mock data)
    let uids = extract_uids_from_samples()
        .expect("Failed to extract UIDs from sample DICOM files");

    // Set up temporary working directory
    let test_base = PathBuf::from("./tmp/dicomweb_integration");
    let _ = std::fs::remove_dir_all(&test_base);
    std::fs::create_dir_all(&test_base).expect("create test base dir");

    // Build Harmony config with DICOMweb endpoint and mock DICOM backend
    let harmony_config = r#"
        [proxy]
        id = "dicomweb-integration-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8091

        [pipelines.dicomweb_bridge]
        description = "DICOMweb to DIMSE bridge with mock backend"
        networks = ["default"]
        endpoints = ["dicomweb"]
        middleware = ["dicomweb_bridge"]
        backends = ["mock_dicom_backend"]

        [endpoints.dicomweb]
        service = "dicomweb"
        [endpoints.dicomweb.options]
        path_prefix = "/dicomweb"

        [backends.mock_dicom_backend]
        service = "mock_dicom"

        [services.dicomweb]
        module = ""
        [services.mock_dicom]
        module = ""

        [middleware_types.dicomweb_bridge]
        module = ""
    "#;

    let app = build_router_with_config(&harmony_config)
        .await
        .expect("Failed to build router");

    TestContext { app, uids }
}

/// Helper to extract UID value from DICOM JSON response
/// Handles both single object (with nested Value array) and array responses
pub fn extract_uid_from_response(data: &serde_json::Value, tag: &str) -> Option<String> {
    data.get(tag)
        .and_then(|v| v.get("Value"))
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Helper to verify requested UID is in response (handles both array and object responses)
pub fn assert_uid_in_response(
    response: &serde_json::Value,
    tag: &str,
    expected_uid: &str,
    context: &str,
) {
    if response.is_array() {
        let arr = response.as_array().unwrap();
        assert!(!arr.is_empty(), "{}: Response array is empty", context);
        assert!(
            arr.iter()
                .any(|item| extract_uid_from_response(item, tag).as_deref() == Some(expected_uid)),
            "{}: Response array does not contain expected {} value {}",
            context,
            tag,
            expected_uid
        );
    } else if response.is_object() {
        let uid_value = extract_uid_from_response(response, tag);
        assert_eq!(
            uid_value.as_deref(),
            Some(expected_uid),
            "{}: Response object does not contain matching {} value",
            context,
            tag
        );
    } else {
        panic!("{}: Expected response to be array or object", context);
    }
}
