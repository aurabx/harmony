//! Integration tests for a C-STORE that a `dicom_scp` endpoint hands to a
//! `dicom_scu` backend in the same pipeline, with no middleware.
//!
//! The backend must send the received instance to the PACS (AURA-2476). Each
//! test calls the DIMSE adapter's `store`, the function the SCP calls for a
//! received C-STORE, and runs the real pipeline against a DICOM SCP in this
//! process that records what it receives.

use dicom_object::InMemDicomObject;
use dimse::scp::QueryProvider;
use dimse::types::{
    dataset_stream_from_vec, BoxDatasetStream, DatasetStream, QueryLevel, RetrieveStream,
};
use dimse::{DimseConfig, DimseScp};
use harmony::adapters::dimse::query_provider::PipelineQueryProvider;
use harmony::config::config::Config;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// The instances the PACS took, in the order they arrived.
type Stored = Arc<Mutex<Vec<InMemDicomObject>>>;

/// A PACS that keeps every instance it is sent and serves nothing.
struct RecordingPacs {
    stored: Stored,
}

#[async_trait::async_trait]
impl QueryProvider for RecordingPacs {
    async fn find(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
        _max_results: u32,
    ) -> dimse::Result<BoxDatasetStream> {
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn locate(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<BoxDatasetStream> {
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn get(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<RetrieveStream> {
        Ok(RetrieveStream::new(dataset_stream_from_vec(Vec::new())))
    }

    async fn store(&self, dataset: DatasetStream) -> dimse::Result<()> {
        let object = dataset.to_object().await?;
        self.stored.lock().unwrap().push(object);
        Ok(())
    }
}

/// A local port that is free now. Nothing listens on it until a caller binds it.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    listener.local_addr().expect("local address").port()
}

fn wait_for_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the PACS did not start listening on port {}", port);
}

/// The adapter reads the process-wide config, so all tests share one config
/// and one PACS.
///
/// The PACS runs on its own thread and runtime. Each `#[tokio::test]` has its
/// own runtime, and a server spawned on one of those would stop when that test
/// ends.
fn pacs() -> &'static Stored {
    static PACS: OnceLock<Stored> = OnceLock::new();
    PACS.get_or_init(|| {
        let stored: Stored = Arc::new(Mutex::new(Vec::new()));
        let pacs_port = free_port();

        let provider = Arc::new(RecordingPacs {
            stored: stored.clone(),
        });
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("PACS runtime");
            let storage_dir = tempfile::tempdir().expect("PACS storage dir");
            let scp = DimseScp::new(
                DimseConfig {
                    local_aet: "TEST_PACS".to_string(),
                    bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                    port: pacs_port,
                    enable_store: true,
                    storage_dir: storage_dir.path().to_path_buf(),
                    ..Default::default()
                },
                provider,
            );
            runtime
                .block_on(scp.run(CancellationToken::new()))
                .expect("PACS stopped");
        });
        wait_for_port(pacs_port);

        harmony::globals::set_config(Arc::new(pipeline_config(pacs_port, free_port())));
        stored
    })
}

fn pipeline_config(pacs_port: u16, down_pacs_port: u16) -> Config {
    let toml = format!(
        r#"
[proxy]
id = "dicom-scp-scu-store-test"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.tcp_config]
bind_address = "127.0.0.1"
bind_port = 11112

# dicom_scp -> dicom_scu, PACS is up
[pipelines.to_pacs]
networks = ["default"]
endpoints = ["scp_to_pacs"]
backends = ["pacs"]
middleware = []

[endpoints.scp_to_pacs]
service = "dicom_scp"
[endpoints.scp_to_pacs.options]
local_aet = "HARMONY_S1"
port = 11113
enable_store = true

[backends.pacs]
service = "dicom_scu"
[backends.pacs.options]
aet = "TEST_PACS"
host = "127.0.0.1"
port = {pacs_port}
local_aet = "HARMONY_S1"

# dicom_scp -> dicom_scu, PACS is down
[pipelines.to_down_pacs]
networks = ["default"]
endpoints = ["scp_to_down_pacs"]
backends = ["down_pacs"]
middleware = []

[endpoints.scp_to_down_pacs]
service = "dicom_scp"
[endpoints.scp_to_down_pacs.options]
local_aet = "HARMONY_S2"
port = 11114
enable_store = true

[backends.down_pacs]
service = "dicom_scu"
[backends.down_pacs.options]
aet = "DOWN_PACS"
host = "127.0.0.1"
port = {down_pacs_port}
local_aet = "HARMONY_S2"

[services.dicom_scp]
module = ""

[services.dicom_scu]
module = ""
"#
    );

    let mut config: Config = toml::from_str(&toml).expect("TOML parse error");
    harmony::config::resolution::resolve_references(&mut config)
        .expect("reference resolution failed");
    config.validate().expect("config validation failed");
    config
}

fn sample_path() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples/dicom/study_1/series_1/CT.1.1.dcm")
}

fn element_bytes(object: &InMemDicomObject, name: &str) -> Vec<u8> {
    object
        .element_by_name(name)
        .unwrap_or_else(|e| panic!("{} missing: {}", name, e))
        .to_bytes()
        .unwrap_or_else(|e| panic!("{} has no bytes: {}", name, e))
        .to_vec()
}

/// Send the sample instance to `pipeline` the way the SCP does, and remove the
/// copy the adapter wrote on this host.
async fn send_sample(pipeline: &str, endpoint: &str) -> dimse::Result<()> {
    let source = dicom_object::open_file(sample_path()).expect("open sample instance");
    let dataset = DatasetStream::from_object(source.into_inner());

    // No storage backend is configured, so the adapter writes its copy here.
    let stored_file = PathBuf::from("./tmp/dimse").join(format!("{}.dcm", dataset.metadata().id));

    let outcome = PipelineQueryProvider::new(pipeline, endpoint)
        .store(dataset)
        .await;
    std::fs::remove_file(&stored_file).expect("remove the adapter's copy");
    outcome
}

#[tokio::test]
async fn dicom_scu_backend_sends_the_received_instance_to_the_pacs() {
    let stored = pacs();

    send_sample("to_pacs", "scp_to_pacs")
        .await
        .expect("the PACS is up, so the C-STORE should succeed");

    let stored = std::mem::take(&mut *stored.lock().unwrap());
    assert_eq!(stored.len(), 1, "the PACS should hold exactly one instance");

    let source = dicom_object::open_file(sample_path()).expect("open sample instance");
    for name in ["SOPInstanceUID", "StudyInstanceUID", "PixelData"] {
        assert_eq!(
            element_bytes(&stored[0], name),
            element_bytes(&source, name),
            "{} changed on the way to the PACS",
            name
        );
    }
}

#[tokio::test]
async fn dicom_scu_backend_reports_a_store_the_pacs_did_not_take() {
    pacs();

    let error = send_sample("to_down_pacs", "scp_to_down_pacs")
        .await
        .expect_err("nothing listens on the PACS port, so the store must fail");

    // The failure must be the connection to the PACS. A body the backend
    // could not read would also fail the store, for the wrong reason.
    let message = error.to_string();
    assert!(
        message.contains("Failed to establish association"),
        "the store should fail at the PACS connection: {}",
        message
    );
}
