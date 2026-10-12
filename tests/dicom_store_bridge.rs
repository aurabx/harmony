//! Integration tests for a C-STORE that leaves a `dicom_scp` endpoint over HTTP.
//!
//! With the `dicom_to_http` middleware and an `http_streaming_backend` the
//! request must carry the DICOM Part 10 file itself. The receiving host
//! cannot use a path on the sending host, and it must never be asked to open
//! one (AURA-2408).
//!
//! Each test calls the DIMSE adapter's `store`, the function the SCP calls for
//! a received C-STORE, and runs the real pipeline against a local HTTP server
//! that records what it receives.
//!
//! A store that the receiving side could not complete must come back from
//! `store` as an error. The SCP answers the sending device from that result, so
//! an `Ok` tells the device it may delete its copy (AURA-2467).

use axum::body::Bytes;
use axum::http::HeaderMap;
use axum::Router;
use dimse::scp::QueryProvider;
use dimse::types::DatasetStream;
use harmony::adapters::dimse::query_provider::PipelineQueryProvider;
use harmony::config::config::Config;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::net::TcpListener;

/// One request as a receiving server saw it.
#[derive(Debug, Clone)]
struct RecordedRequest {
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

type Recorded = Arc<Mutex<Vec<RecordedRequest>>>;

/// What a receiving Harmony answers when its PACS took the instance.
const STORED_REPLY: &str = "{\"operation\":\"store\",\"success\":true}\n";

/// What a receiving Harmony answers when its PACS did not take the instance.
/// The HTTP status is still 200. Only the line says the store failed.
const NOT_STORED_REPLY: &str = "{\"operation\":\"store\",\"success\":false,\"remote_aet\":\"PACS\",\"host\":\"127.0.0.1\",\"port\":4242,\"error\":\"C-STORE to PACS@127.0.0.1:4242 failed: connection refused\",\"_stream_sequence\":1}\n";

/// Start a server that records every request and answers the way a receiving
/// Harmony answers a C-STORE: HTTP 200 with `reply` as one NDJSON line.
async fn start_recording_receiver(reply: &'static str) -> (u16, Recorded) {
    let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
    let recorded_for_handler = recorded.clone();
    let app = Router::new().fallback(move |headers: HeaderMap, body: Bytes| {
        let recorded = recorded_for_handler.clone();
        async move {
            let headers = headers
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
                })
                .collect();
            recorded.lock().unwrap().push(RecordedRequest {
                headers,
                body: body.to_vec(),
            });
            ([("content-type", "application/x-ndjson")], reply)
        }
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, recorded)
}

/// The receiver that each pipeline under test sends to.
struct Bridges {
    streaming: Recorded,
    non_streaming: Recorded,
    /// Answers every store with `NOT_STORED_REPLY`.
    failing: Recorded,
}

/// A local port with nothing listening on it, standing in for a PACS that is
/// down.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    listener.local_addr().expect("local address").port()
}

/// The adapter reads the process-wide config, so all tests share one config
/// with one pipeline per shape.
///
/// The receivers run on their own thread and runtime. Each `#[tokio::test]`
/// has its own runtime, and a server spawned on one of those would stop when
/// that test ends.
fn bridges() -> &'static Bridges {
    static BRIDGES: OnceLock<Bridges> = OnceLock::new();
    BRIDGES.get_or_init(|| {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("receiver runtime");
            runtime.block_on(async move {
                let streaming = start_recording_receiver(STORED_REPLY).await;
                let non_streaming = start_recording_receiver(STORED_REPLY).await;
                let failing = start_recording_receiver(NOT_STORED_REPLY).await;
                started_tx
                    .send((streaming, non_streaming, failing))
                    .expect("report receiver ports");
                std::future::pending::<()>().await;
            });
        });
        let (
            (streaming_port, streaming),
            (non_streaming_port, non_streaming),
            (failing_port, failing),
        ) = started_rx.recv().expect("receivers should start");

        let config = bridge_config(
            streaming_port,
            non_streaming_port,
            failing_port,
            closed_port(),
        );
        harmony::globals::set_config(Arc::new(config));
        Bridges {
            streaming,
            non_streaming,
            failing,
        }
    })
}

fn bridge_config(
    streaming_port: u16,
    non_streaming_port: u16,
    failing_port: u16,
    down_pacs_port: u16,
) -> Config {
    let toml = format!(
        r#"
[proxy]
id = "dicom-store-bridge-test"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.tcp_config]
bind_address = "127.0.0.1"
bind_port = 11112

# dicom_scp -> dicom_to_http -> http_streaming_backend
[pipelines.streaming_bridge]
networks = ["default"]
endpoints = ["scp_streaming"]
backends = ["streaming_receiver"]
middleware = ["dicom_to_http"]

[endpoints.scp_streaming]
service = "dicom_scp"
[endpoints.scp_streaming.options]
local_aet = "HARMONY_A1"
port = 11113
enable_store = true

[backends.streaming_receiver]
service = "http_streaming_backend"
[backends.streaming_receiver.options]
base_url = "http://127.0.0.1:{streaming_port}"
path_prefix = "/store"

# dicom_scp -> dicom_to_http -> http
[pipelines.non_streaming_bridge]
networks = ["default"]
endpoints = ["scp_non_streaming"]
backends = ["non_streaming_receiver"]
middleware = ["dicom_to_http"]

[endpoints.scp_non_streaming]
service = "dicom_scp"
[endpoints.scp_non_streaming.options]
local_aet = "HARMONY_A2"
port = 11114
enable_store = true

[targets.non_streaming_target]
connection.host = "127.0.0.1"
connection.port = {non_streaming_port}
connection.protocol = "http"

[backends.non_streaming_receiver]
service = "http"
target_ref = "non_streaming_target"

# dicom_scp -> dicom_to_http -> http_streaming_backend, receiver cannot store
[pipelines.failing_bridge]
networks = ["default"]
endpoints = ["scp_failing"]
backends = ["failing_receiver"]
middleware = ["dicom_to_http"]

[endpoints.scp_failing]
service = "dicom_scp"
[endpoints.scp_failing.options]
local_aet = "HARMONY_A3"
port = 11115
enable_store = true

[backends.failing_receiver]
service = "http_streaming_backend"
[backends.failing_receiver.options]
base_url = "http://127.0.0.1:{failing_port}"
path_prefix = "/store"

# dicom_scp -> dicom_scu_streaming, no HTTP hop, PACS is down
[pipelines.single_node]
networks = ["default"]
endpoints = ["scp_single_node"]
backends = ["down_pacs"]
middleware = []

[endpoints.scp_single_node]
service = "dicom_scp"
[endpoints.scp_single_node.options]
local_aet = "HARMONY_A4"
port = 11116
enable_store = true

[backends.down_pacs]
service = "dicom_scu_streaming"
[backends.down_pacs.options]
aet = "DOWN_PACS"
host = "127.0.0.1"
port = {down_pacs_port}
local_aet = "HARMONY_A4"

[middleware.dicom_to_http]
type = "dicom_to_http"

[services.dicom_scp]
module = ""

[services.dicom_scu_streaming]
module = ""

[services.http]
module = ""

[services.http_streaming_backend]
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

fn element_bytes(object: &dicom_object::InMemDicomObject, name: &str) -> Vec<u8> {
    object
        .element_by_name(name)
        .unwrap_or_else(|e| panic!("{} missing: {}", name, e))
        .to_bytes()
        .unwrap_or_else(|e| panic!("{} has no bytes: {}", name, e))
        .to_vec()
}

/// What one C-STORE through a pipeline produced.
struct StoreRun {
    /// The single request the receiver got.
    request: RecordedRequest,
    /// The copy the adapter wrote on the sending host.
    stored_copy: Vec<u8>,
    /// Where that copy was, on the sending host.
    stored_path: String,
}

/// Send the sample instance to `pipeline` the way the SCP does. Returns what
/// `store` returned and where the adapter wrote its copy of the instance.
async fn send_sample(pipeline: &str, endpoint: &str) -> (dimse::Result<()>, PathBuf) {
    let source = dicom_object::open_file(sample_path()).expect("open sample instance");
    let dataset = DatasetStream::from_object(source.into_inner());

    // No storage backend is configured, so the adapter writes its copy here.
    let stored_file = PathBuf::from("./tmp/dimse").join(format!("{}.dcm", dataset.metadata().id));

    let outcome = PipelineQueryProvider::new(pipeline, endpoint)
        .store(dataset)
        .await;
    (outcome, stored_file)
}

/// Send the sample instance to a pipeline whose receiver stores it.
async fn store_sample(pipeline: &str, endpoint: &str, recorded: &Recorded) -> StoreRun {
    let (outcome, stored_file) = send_sample(pipeline, endpoint).await;
    outcome.unwrap_or_else(|e| panic!("C-STORE through '{}' should succeed: {}", pipeline, e));

    let stored_copy = std::fs::read(&stored_file).expect("the adapter's copy of the instance");
    std::fs::remove_file(&stored_file).expect("remove the adapter's copy");

    let mut recorded = recorded.lock().unwrap();
    assert_eq!(recorded.len(), 1, "receiver should get exactly one request");
    StoreRun {
        request: recorded.remove(0),
        stored_copy,
        stored_path: stored_file.to_string_lossy().into_owned(),
    }
}

/// The request must be the instance itself, described by its headers, with
/// nothing in it that refers to the sending host's disk.
fn assert_request_is_the_instance(run: &StoreRun) {
    let request = &run.request;
    assert_eq!(
        request.body, run.stored_copy,
        "the request body must be the Part 10 file the adapter stored"
    );

    let source = dicom_object::open_file(sample_path()).expect("open sample instance");
    let received = dicom_object::from_reader(std::io::Cursor::new(&request.body))
        .expect("the request body must parse as a DICOM Part 10 file");
    for name in ["SOPInstanceUID", "StudyInstanceUID", "PixelData"] {
        assert_eq!(
            element_bytes(&received, name),
            element_bytes(&source, name),
            "{} changed on the way to the receiver",
            name
        );
    }

    let header = |name: &str| request.headers.get(name).map(String::as_str);
    assert_eq!(header("content-type"), Some("application/dicom"));
    assert_eq!(header("x-dimse-op"), Some("store"));
    assert_eq!(
        header("x-dicom-transfer-syntax"),
        Some(received.meta().transfer_syntax())
    );
    let uid_header = |name: &str| {
        let value = String::from_utf8(element_bytes(&received, name)).expect("UID is text");
        value.trim_end_matches(['\0', ' ']).to_string()
    };
    assert_eq!(
        header("x-dicom-sop-class-uid"),
        Some(uid_header("SOPClassUID").as_str())
    );
    assert_eq!(
        header("x-dicom-sop-instance-uid"),
        Some(uid_header("SOPInstanceUID").as_str())
    );

    let file_name = run
        .stored_path
        .rsplit('/')
        .next()
        .expect("stored path has a file name");
    for (name, value) in &request.headers {
        assert!(
            !value.contains(file_name),
            "header {} carries the sender's local path: {}",
            name,
            value
        );
    }
}

#[tokio::test]
async fn streaming_bridge_sends_the_part10_file_and_no_path() {
    let bridges = bridges();

    let run = store_sample("streaming_bridge", "scp_streaming", &bridges.streaming).await;

    assert_request_is_the_instance(&run);
}

#[tokio::test]
async fn http_backend_pipeline_keeps_its_json_body() {
    // The `http` backend does not send the headers `dicom_to_http` sets, so it
    // cannot bridge a C-STORE to another Harmony. It posts the DICOM JSON, as
    // it did before the instance could travel inline.
    let bridges = bridges();

    let run = store_sample(
        "non_streaming_bridge",
        "scp_non_streaming",
        &bridges.non_streaming,
    )
    .await;

    let body: Value = serde_json::from_slice(&run.request.body).unwrap_or_else(|e| {
        panic!(
            "the receiver must get JSON, got {} bytes that are not JSON: {}",
            run.request.body.len(),
            e
        )
    });
    let patient_id = json!({ "vr": "LO", "Value": ["PID156695"] });
    assert_eq!(body.pointer("/dataset/00100020"), Some(&patient_id));
    assert!(body.get("part10_b64").is_none());
    assert!(body.get("body_b64").is_none());
    assert!(
        !run.request.headers.contains_key("x-dimse-op"),
        "the http backend does not send the headers dicom_to_http sets"
    );
}

#[tokio::test]
async fn streaming_bridge_reports_a_store_the_receiver_could_not_complete() {
    let bridges = bridges();

    let (outcome, stored_file) = send_sample("failing_bridge", "scp_failing").await;
    std::fs::remove_file(&stored_file).expect("remove the adapter's copy");

    // The instance did reach the receiver, so the error below comes from the
    // receiver's answer and not from a request that never left.
    let received = std::mem::take(&mut *bridges.failing.lock().unwrap());
    assert_eq!(received.len(), 1, "receiver should get exactly one request");

    let error = outcome.expect_err(
        "the receiver answered success:false, so the sending device must not be told the instance is stored",
    );
    assert!(
        error.to_string().contains("connection refused"),
        "the error should carry the receiver's reason: {}",
        error
    );
}

#[tokio::test]
async fn single_node_pipeline_reports_a_store_the_pacs_did_not_take() {
    // No HTTP hop: `dicom_scu_streaming` reports the failed C-STORE as
    // `dicom_status` in the response metadata of the same process.
    bridges();

    let (outcome, stored_file) = send_sample("single_node", "scp_single_node").await;
    std::fs::remove_file(&stored_file).expect("remove the adapter's copy");

    outcome.expect_err("nothing listens on the PACS port, so the store must fail");
}
