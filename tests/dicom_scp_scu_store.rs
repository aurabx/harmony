//! Integration tests for a C-STORE that a `dicom_scp` endpoint hands to a
//! `dicom_scu` backend in the same pipeline.
//!
//! The backend must send the received instance to the PACS (AURA-2476). Each
//! test calls the DIMSE adapter's `store`, the function the SCP calls for a
//! received C-STORE, and runs the real pipeline against a DICOM SCP in this
//! process that records what it receives.
//!
//! An instance with compressed pixel data must reach the PACS in its own
//! transfer syntax, with its pixel fragments unchanged (AURA-2470).
//!
//! An `ip_allow` policy on a `dicom_scp` pipeline must see the SCU's address,
//! so it admits a client inside the allowlist and denies one outside it.

use dicom_core::value::{PixelFragmentSequence, Value};
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
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

/// An instance the PACS took, with the transfer syntax it arrived in.
struct StoredInstance {
    transfer_syntax: Option<String>,
    object: InMemDicomObject,
}

/// The instances the PACS took, in the order they arrived.
type Stored = Arc<Mutex<Vec<StoredInstance>>>;

/// A PACS that keeps every instance it is sent and serves nothing.
struct RecordingPacs {
    stored: Stored,
}

#[async_trait::async_trait]
impl QueryProvider for RecordingPacs {
    async fn find(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
        _max_results: u32,
    ) -> dimse::Result<BoxDatasetStream> {
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn locate(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<BoxDatasetStream> {
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn get(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<RetrieveStream> {
        Ok(RetrieveStream::new(dataset_stream_from_vec(Vec::new())))
    }

    async fn store(
        &self,
        _peer_addr: std::net::SocketAddr,
        dataset: DatasetStream,
    ) -> dimse::Result<()> {
        let instance = StoredInstance {
            transfer_syntax: dataset.metadata().transfer_syntax.clone(),
            object: dataset.to_object().await?,
        };
        self.stored.lock().unwrap().push(instance);
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

# dicom_scp -> dicom, the legacy name of dicom_scu, PACS is up
[pipelines.to_pacs_legacy_name]
networks = ["default"]
endpoints = ["scp_to_pacs_legacy_name"]
backends = ["pacs_legacy_name"]
middleware = []

[endpoints.scp_to_pacs_legacy_name]
service = "dicom_scp"
[endpoints.scp_to_pacs_legacy_name.options]
local_aet = "HARMONY_S3"
port = 11115
enable_store = true

[backends.pacs_legacy_name]
service = "dicom"
[backends.pacs_legacy_name.options]
aet = "TEST_PACS"
host = "127.0.0.1"
port = {pacs_port}
local_aet = "HARMONY_S3"

# dicom_scp -> dicom_scu_streaming, PACS is up
[pipelines.to_pacs_streaming]
networks = ["default"]
endpoints = ["scp_to_pacs_streaming"]
backends = ["pacs_streaming"]
middleware = []

[endpoints.scp_to_pacs_streaming]
service = "dicom_scp"
[endpoints.scp_to_pacs_streaming.options]
local_aet = "HARMONY_S4"
port = 11116
enable_store = true

[backends.pacs_streaming]
service = "dicom_scu_streaming"
[backends.pacs_streaming.options]
aet = "TEST_PACS"
host = "127.0.0.1"
port = {pacs_port}
local_aet = "HARMONY_S4"

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

# dicom_scp -> dicom_scu behind an IP allowlist, PACS is up
[pipelines.to_pacs_ip_gated]
networks = ["default"]
endpoints = ["scp_to_pacs_ip_gated"]
backends = ["pacs_ip_gated"]
middleware = ["ip_gate"]

[endpoints.scp_to_pacs_ip_gated]
service = "dicom_scp"
[endpoints.scp_to_pacs_ip_gated.options]
local_aet = "HARMONY_S5"
port = 11117
enable_find = true
enable_get = true
enable_store = true

[backends.pacs_ip_gated]
service = "dicom_scu"
[backends.pacs_ip_gated.options]
aet = "TEST_PACS"
host = "127.0.0.1"
port = {pacs_port}
local_aet = "HARMONY_S5"

# dicom_scp with no backend behind an IP allowlist. The adapter's copy is
# the stored instance.
[pipelines.no_backend_ip_gated]
networks = ["default"]
endpoints = ["scp_no_backend_ip_gated"]
backends = []
middleware = ["ip_gate"]

[endpoints.scp_no_backend_ip_gated]
service = "dicom_scp"
[endpoints.scp_no_backend_ip_gated.options]
local_aet = "HARMONY_S6"
port = 11118
enable_store = true

[middleware.ip_gate]
type = "policies"
[middleware.ip_gate.options]
policies = ["ip_gate_policy"]

[policies.ip_gate_policy]
id = "ip_gate_policy"
enabled = true
rules = ["modality_network"]

[rules.modality_network]
id = "modality_network"
type = "ip_allow"
weight = 100
enabled = true
[rules.modality_network.options]
ip_addresses = ["10.0.0.0/8"]

[middleware_types.policies]
module = ""

[services.dicom_scp]
module = ""

[services.dicom_scu]
module = ""

[services.dicom]
module = ""

[services.dicom_scu_streaming]
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

/// The sample instance as the SCP hands it to the adapter.
fn sample_instance() -> DatasetStream {
    let source = dicom_object::open_file(sample_path()).expect("open sample instance");
    DatasetStream::from_object(source.into_inner())
}

/// Where the adapter writes its copy of `dataset`. No storage backend is
/// configured, so it uses `./tmp/dimse`.
fn adapter_copy_path(dataset: &DatasetStream) -> PathBuf {
    PathBuf::from("./tmp/dimse").join(format!("{}.dcm", dataset.metadata().id))
}

/// Send the sample instance to `pipeline` the way the SCP does.
async fn send_sample(pipeline: &str, endpoint: &str) -> dimse::Result<()> {
    send(pipeline, endpoint, sample_instance()).await
}

/// Hand `dataset` to `pipeline` the way the SCP does.
async fn send(pipeline: &str, endpoint: &str, dataset: DatasetStream) -> dimse::Result<()> {
    // A loopback peer, as the SCP would pass for a local SCU.
    send_from("127.0.0.1:40000", pipeline, endpoint, dataset).await
}

/// Hand `dataset` to `pipeline` as if the SCU connected from `peer`.
async fn store_from(
    peer: &str,
    pipeline: &str,
    endpoint: &str,
    dataset: DatasetStream,
) -> dimse::Result<()> {
    let peer: std::net::SocketAddr = peer.parse().expect("valid peer address");
    PipelineQueryProvider::new(pipeline, endpoint)
        .store(peer, dataset)
        .await
}

/// Hand `dataset` to `pipeline` as if the SCU connected from `peer`.
///
/// Whatever the outcome, the adapter must have removed its copy of the
/// instance by the time the store returns. Every pipeline sent through here
/// has a backend, or denies the store.
async fn send_from(
    peer: &str,
    pipeline: &str,
    endpoint: &str,
    dataset: DatasetStream,
) -> dimse::Result<()> {
    let stored_file = adapter_copy_path(&dataset);
    let outcome = store_from(peer, pipeline, endpoint, dataset).await;
    assert!(
        !stored_file.exists(),
        "the adapter's copy must be removed when the store finishes ({:?}): {}",
        outcome,
        stored_file.display()
    );
    outcome
}

/// Send the sample instance through `pipeline` and check that the PACS took
/// exactly that instance, unchanged.
async fn assert_pipeline_sends_the_instance_to_the_pacs(pipeline: &str, endpoint: &str) {
    let stored = pacs();

    let dataset = sample_instance();
    let copy = adapter_copy_path(&dataset);
    send(pipeline, endpoint, dataset)
        .await
        .expect("the PACS is up, so the C-STORE should succeed");
    assert!(
        !copy.exists(),
        "a successful store must remove the adapter's copy"
    );

    let stored = std::mem::take(&mut *stored.lock().unwrap());
    assert_eq!(stored.len(), 1, "the PACS should hold exactly one instance");

    let source = dicom_object::open_file(sample_path()).expect("open sample instance");
    for name in ["SOPInstanceUID", "StudyInstanceUID", "PixelData"] {
        assert_eq!(
            element_bytes(&stored[0].object, name),
            element_bytes(&source, name),
            "{} changed on the way to the PACS",
            name
        );
    }
}

const JPEG_LOSSLESS_SV1: &str = "1.2.840.10008.1.2.4.70";

/// The fragments are not a real JPEG stream. Nothing on the path decodes
/// them, and they only have to arrive unchanged. Each has an even length, as
/// DICOM requires of a fragment.
fn compressed_fragments() -> Vec<Vec<u8>> {
    vec![
        vec![0xFF, 0xD8, 0x01, 0x02, 0x03, 0x04, 0xFF, 0xD9],
        vec![0xFF, 0xD8, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0xFF, 0xD9],
    ]
}

/// A JPEG Lossless instance as the SCP hands it to the adapter: the parsed
/// object with encapsulated pixel data, and the negotiated transfer syntax
/// in the metadata.
fn compressed_instance() -> DatasetStream {
    let mut object = InMemDicomObject::new_empty();
    for (tag, value) in [
        (tags::SOP_CLASS_UID, uids::SECONDARY_CAPTURE_IMAGE_STORAGE),
        (tags::SOP_INSTANCE_UID, "1.2.826.0.1.3680043.9.7133.2470.2"),
        (tags::STUDY_INSTANCE_UID, "1.2.826.0.1.3680043.9.7133.2470.3"),
        (tags::SERIES_INSTANCE_UID, "1.2.826.0.1.3680043.9.7133.2470.4"),
    ] {
        object.put(DataElement::new(tag, VR::UI, PrimitiveValue::from(value)));
    }
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        Value::PixelSequence(PixelFragmentSequence::new_fragments(compressed_fragments())),
    ));

    let mut dataset = DatasetStream::from_object(object);
    dataset.metadata_mut().transfer_syntax = Some(JPEG_LOSSLESS_SV1.to_string());
    dataset
}

// The tests below that are marked `serial` share the PACS and count what it
// holds, so they must not overlap.

/// Send the compressed instance through `pipeline` and check that the PACS
/// took it in JPEG Lossless with the same pixel fragments.
async fn assert_pipeline_keeps_a_compressed_instance(pipeline: &str, endpoint: &str) {
    let stored = pacs();

    send(pipeline, endpoint, compressed_instance())
        .await
        .expect("the PACS accepts every transfer syntax, so the C-STORE should succeed");

    let stored = std::mem::take(&mut *stored.lock().unwrap());
    assert_eq!(stored.len(), 1, "the PACS should hold exactly one instance");
    assert_eq!(
        stored[0].transfer_syntax.as_deref(),
        Some(JPEG_LOSSLESS_SV1),
        "the transfer syntax changed on the way to the PACS"
    );

    let pixel_data = stored[0]
        .object
        .element(tags::PIXEL_DATA)
        .expect("PixelData missing");
    match pixel_data.value() {
        Value::PixelSequence(sequence) => assert_eq!(
            sequence.fragments(),
            compressed_fragments().as_slice(),
            "the pixel fragments changed on the way to the PACS"
        ),
        other => panic!("PixelData is no longer encapsulated: {:?}", other),
    }
}

#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn dicom_scu_backend_sends_a_compressed_instance_in_its_own_transfer_syntax() {
    assert_pipeline_keeps_a_compressed_instance("to_pacs", "scp_to_pacs").await;
}

#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn dicom_scu_streaming_backend_sends_a_compressed_instance_in_its_own_transfer_syntax() {
    assert_pipeline_keeps_a_compressed_instance("to_pacs_streaming", "scp_to_pacs_streaming")
        .await;
}

#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn dicom_scu_backend_sends_an_uncompressed_instance_in_its_own_transfer_syntax() {
    let stored = pacs();

    send_sample("to_pacs", "scp_to_pacs")
        .await
        .expect("the PACS is up, so the C-STORE should succeed");

    // The sample is sent with no recorded transfer syntax, so the adapter
    // writes it as Explicit VR Little Endian. The backend reads that from
    // the file and proposes it first.
    let stored = std::mem::take(&mut *stored.lock().unwrap());
    assert_eq!(stored.len(), 1, "the PACS should hold exactly one instance");
    assert_eq!(
        stored[0].transfer_syntax.as_deref(),
        Some(uids::EXPLICIT_VR_LITTLE_ENDIAN)
    );
}

#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn dicom_scu_backend_sends_the_received_instance_to_the_pacs() {
    assert_pipeline_sends_the_instance_to_the_pacs("to_pacs", "scp_to_pacs").await;
}

#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn backend_named_dicom_sends_the_received_instance_to_the_pacs() {
    // `dicom` is the legacy name of the `dicom_scu` service.
    assert_pipeline_sends_the_instance_to_the_pacs(
        "to_pacs_legacy_name",
        "scp_to_pacs_legacy_name",
    )
    .await;
}

#[tokio::test]
async fn dicom_scu_backend_reports_a_store_the_pacs_did_not_take() {
    pacs();

    let dataset = sample_instance();
    let copy = adapter_copy_path(&dataset);
    let error = send("to_down_pacs", "scp_to_down_pacs", dataset)
        .await
        .expect_err("nothing listens on the PACS port, so the store must fail");
    assert!(
        !copy.exists(),
        "a failed store must remove the adapter's copy"
    );

    // The failure must be the connection to the PACS. A body the backend
    // could not read would also fail the store, for the wrong reason.
    let message = error.to_string();
    assert!(
        message.contains("Failed to establish association"),
        "the store should fail at the PACS connection: {}",
        message
    );
}

/// Send the sample instance to `pipeline` as if the SCU connected from `peer`.
async fn send_sample_from(peer: &str, pipeline: &str, endpoint: &str) -> dimse::Result<()> {
    send_from(peer, pipeline, endpoint, sample_instance()).await
}

/// An `ip_allow` rule on a `dicom_scp` pipeline sees the SCU's address. Before
/// the DIMSE adapter recorded it, the client IP was unknown and the allowlist
/// denied every C-STORE.
#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn ip_allowlist_admits_a_dimse_client_inside_it() {
    let stored = pacs();

    send_sample_from("10.1.2.3:40000", "to_pacs_ip_gated", "scp_to_pacs_ip_gated")
        .await
        .expect("10.1.2.3 is inside the allowlist, so the C-STORE should succeed");

    let stored = std::mem::take(&mut *stored.lock().unwrap());
    assert_eq!(stored.len(), 1, "the PACS should hold exactly one instance");
}

#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn ip_allowlist_denies_a_dimse_client_outside_it() {
    let stored = pacs();

    // The adapter writes its copy before the policy runs, so a denied client
    // must not leave it behind.
    let dataset = sample_instance();
    let copy = adapter_copy_path(&dataset);
    let error = send_from(
        "203.0.113.9:40000",
        "to_pacs_ip_gated",
        "scp_to_pacs_ip_gated",
        dataset,
    )
    .await
    .expect_err("203.0.113.9 is outside the allowlist, so the C-STORE must fail");
    assert!(
        !copy.exists(),
        "a denied store must remove the adapter's copy"
    );

    // An unknown address would also be denied, for the wrong reason. The
    // policy must have compared the client's address against the allowlist.
    let message = error.to_string();
    assert!(
        message.contains("not in the IP allowlist"),
        "the policy should deny the client by its address: {}",
        message
    );
    assert!(
        std::mem::take(&mut *stored.lock().unwrap()).is_empty(),
        "a denied C-STORE must not reach the PACS"
    );
}

/// A pipeline with no backend keeps the adapter's copy as the stored
/// instance, but only once the store succeeds.
#[tokio::test]
async fn no_backend_pipeline_keeps_the_copy_of_an_admitted_store() {
    pacs();

    let dataset = sample_instance();
    let copy = adapter_copy_path(&dataset);

    let outcome = store_from(
        "10.1.2.3:40000",
        "no_backend_ip_gated",
        "scp_no_backend_ip_gated",
        dataset,
    )
    .await;
    let kept = copy.exists();
    if kept {
        std::fs::remove_file(&copy).expect("remove the kept copy");
    }

    outcome.expect("10.1.2.3 is inside the allowlist, so the C-STORE should succeed");
    assert!(
        kept,
        "a successful store with no backend must keep the adapter's copy"
    );
}

#[tokio::test]
async fn no_backend_pipeline_removes_the_copy_of_a_denied_store() {
    pacs();

    let dataset = sample_instance();
    let copy = adapter_copy_path(&dataset);

    let error = send_from(
        "203.0.113.9:40000",
        "no_backend_ip_gated",
        "scp_no_backend_ip_gated",
        dataset,
    )
    .await
    .expect_err("203.0.113.9 is outside the allowlist, so the C-STORE must fail");
    assert!(
        error.to_string().contains("not in the IP allowlist"),
        "the policy should deny the client by its address: {}",
        error
    );
    assert!(
        !copy.exists(),
        "a denied store must remove the adapter's copy"
    );
}

/// Run `operation` from an address inside and one outside the allowlist on
/// the IP-gated pipeline. The PACS serves no queries, so an admitted request
/// may still fail at the backend. It must not fail at the policy.
async fn assert_ip_allowlist_gates<F, Fut, T>(operation: F)
where
    F: Fn(std::net::SocketAddr) -> Fut,
    Fut: std::future::Future<Output = dimse::Result<T>>,
{
    pacs();

    if let Err(error) = operation("10.1.2.3:40000".parse().unwrap()).await {
        let message = error.to_string();
        assert!(
            !message.contains("allowlist"),
            "10.1.2.3 is inside the allowlist and must pass the policy: {}",
            message
        );
    }

    let message = match operation("203.0.113.9:40000".parse().unwrap()).await {
        Ok(_) => panic!("203.0.113.9 is outside the allowlist, so the request must fail"),
        Err(error) => error.to_string(),
    };
    assert!(
        message.contains("not in the IP allowlist"),
        "the policy should deny the client by its address: {}",
        message
    );
}

fn gated_provider() -> PipelineQueryProvider {
    PipelineQueryProvider::new("to_pacs_ip_gated", "scp_to_pacs_ip_gated")
}

fn study_parameters() -> HashMap<String, String> {
    HashMap::from([("0020000D".to_string(), "1.2.3".to_string())])
}

#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn ip_allowlist_gates_a_dimse_c_find() {
    assert_ip_allowlist_gates(|peer| async move {
        gated_provider()
            .find(peer, QueryLevel::Study, &study_parameters(), 0)
            .await
            .map(|_| ())
    })
    .await;
}

/// C-MOVE runs through the same `get` call on the provider.
#[tokio::test]
#[serial_test::serial(recording_pacs)]
async fn ip_allowlist_gates_a_dimse_c_get() {
    assert_ip_allowlist_gates(|peer| async move {
        gated_provider()
            .get(peer, QueryLevel::Study, &study_parameters())
            .await
            .map(|_| ())
    })
    .await;
}
