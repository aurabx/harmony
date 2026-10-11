//! Integration test for a C-STORE bridged to another Harmony over HTTP.
//!
//! A `dicom_scp` endpoint with the `dicom_to_http` middleware and an
//! `http_streaming_backend` forwards each stored instance to a second
//! Harmony. The request must carry the DICOM Part 10 file itself. The
//! receiving host cannot use a path on the sending host, and it must never
//! be asked to open one (AURA-2408).
//!
//! The test runs the real sending pipeline against a local HTTP server that
//! records what it receives.

use axum::body::Bytes;
use axum::http::HeaderMap;
use axum::Router;
use base64::Engine;
use futures_util::StreamExt;
use harmony::config::config::Config;
use harmony::models::envelope::command::{Command, Operation};
use harmony::models::protocol::{Protocol, ProtocolCtx};
use harmony::pipeline::PipelineExecutor;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// One request as the receiving server saw it.
#[derive(Debug, Clone)]
struct RecordedRequest {
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

type Recorded = Arc<Mutex<Vec<RecordedRequest>>>;

/// Start a server that records every request and answers the way the
/// receiving Harmony answers a successful C-STORE.
async fn start_recording_receiver() -> (u16, Recorded) {
    let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
    let recorded_for_handler = recorded.clone();
    let app = Router::new().fallback(
        move |uri: axum::http::Uri, headers: HeaderMap, body: Bytes| {
            let recorded = recorded_for_handler.clone();
            async move {
                let headers = headers
                    .iter()
                    .filter_map(|(name, value)| {
                        Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
                    })
                    .collect();
                recorded.lock().unwrap().push(RecordedRequest {
                    path: uri.path().to_string(),
                    headers,
                    body: body.to_vec(),
                });
                axum::Json(json!({ "operation": "store", "success": true }))
            }
        },
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, recorded)
}

fn sending_config(receiver_port: u16) -> Config {
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

[pipelines.store_pipeline]
description = "C-STORE to another Harmony over HTTP"
networks = ["default"]
endpoints = ["store_scp"]
backends = ["harmony_b_store"]
middleware = ["dicom_to_http"]

[endpoints.store_scp]
service = "dicom_scp"
[endpoints.store_scp.options]
local_aet = "HARMONY_A"
enable_store = true

[middleware.dicom_to_http]
type = "dicom_to_http"

[backends.harmony_b_store]
service = "http_streaming_backend"
[backends.harmony_b_store.options]
base_url = "http://127.0.0.1:{receiver_port}"
path_prefix = "/store"

[services.dicom_scp]
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

#[tokio::test]
async fn bridged_store_sends_the_part10_file_and_no_path() {
    let (receiver_port, recorded) = start_recording_receiver().await;
    let config = sending_config(receiver_port);
    let pipeline = config
        .pipelines
        .get("store_pipeline")
        .expect("pipeline present")
        .clone();
    assert!(PipelineExecutor::is_streaming_pipeline(&pipeline, &config));

    let sample = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples/dicom/study_1/series_1/CT.1.1.dcm");
    let part10 = std::fs::read(&sample).expect("read sample instance");
    let source = dicom_object::open_file(&sample).expect("open sample instance");
    let transfer_syntax = source.meta().transfer_syntax().to_string();

    // The body the DIMSE adapter builds for a C-STORE on a streaming pipeline.
    let local_path = "/var/harmony-a/dimse/5f0c.dcm";
    let body = json!({
        "command": {
            "message_id": 1,
            "sop_class_uid": "1.2.840.10008.5.1.4.1.1.2",
            "priority": "MEDIUM",
            "direction": "REQUEST"
        },
        "dataset": { "00100020": { "vr": "LO", "Value": ["PID1"] } },
        "file": local_path,
        "body_b64": base64::engine::general_purpose::STANDARD.encode(&part10),
        "transfer_syntax_uid": transfer_syntax,
    });

    let mut meta = HashMap::new();
    meta.insert("protocol".to_string(), "dimse".to_string());
    meta.insert("SOPInstanceUID".to_string(), "1.2.3.4.5".to_string());
    let ctx = ProtocolCtx {
        protocol: Protocol::Dimse,
        payload: serde_json::to_vec(&body).unwrap(),
        meta,
        attrs: json!({}),
        command: Some(Command::new(Operation::Store)),
    };

    let endpoint = config.endpoints.get("store_scp").expect("endpoint present");
    let service = endpoint.resolve_service().expect("endpoint service resolves");
    let envelope = service
        .build_protocol_envelope(ctx.clone(), endpoint.options.as_ref().unwrap_or(&HashMap::new()))
        .await
        .expect("endpoint builds envelope");

    let responses: Vec<_> =
        PipelineExecutor::execute_streaming(envelope, "store_pipeline", &pipeline, &config, &ctx)
            .await
            .expect("pipeline should reach the receiver")
            .collect()
            .await;
    assert_eq!(responses.len(), 1, "one store response expected");
    responses[0].as_ref().expect("store response should be Ok");

    let recorded = recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1, "receiver should get exactly one request");
    let request = &recorded[0];

    assert_eq!(request.path, "/store");
    assert_eq!(request.body, part10, "the request body must be the Part 10 file");
    let local_path_on_the_wire = request
        .body
        .windows(local_path.len())
        .any(|window| window == local_path.as_bytes());
    assert!(!local_path_on_the_wire, "the sender's local path must not be sent");

    let header = |name: &str| request.headers.get(name).map(String::as_str);
    assert_eq!(header("content-type"), Some("application/dicom"));
    assert_eq!(header("x-dimse-op"), Some("store"));
    assert_eq!(header("x-dicom-transfer-syntax"), Some(transfer_syntax.as_str()));
    assert_eq!(header("x-dicom-sop-class-uid"), Some("1.2.840.10008.5.1.4.1.1.2"));
    assert_eq!(header("x-dicom-sop-instance-uid"), Some("1.2.3.4.5"));
}
