//! Integration tests for request bodies rewritten by middleware.
//!
//! Middleware opt in to replacing the request body by writing
//! `original_data`. The executor must send that body to the backend, and must
//! send the received bytes unchanged when no middleware replaced them.
//! Each test runs a real pipeline against a local HTTP server that records
//! what it receives.

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, Uri};
use axum::Router;
use harmony::config::config::Config;
use harmony::models::envelope::command::{Command, Operation};
use harmony::models::envelope::envelope::RequestEnvelope;
use harmony::models::protocol::{Protocol, ProtocolCtx};
use harmony::pipeline::PipelineExecutor;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio::net::TcpListener;

/// One request as the upstream server saw it.
#[derive(Debug, Clone)]
struct RecordedRequest {
    method: Method,
    path: String,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl RecordedRequest {
    fn json_body(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "backend body is not JSON ({}): {:?}",
                e,
                String::from_utf8_lossy(&self.body)
            )
        })
    }
}

type Recorded = Arc<Mutex<Vec<RecordedRequest>>>;

/// Body the recording backend returns for every request.
fn backend_response_body() -> Value {
    json!({ "backend": "reached", "patient": { "id": "123", "name": "Doe" } })
}

/// Start an upstream server that records every request and answers with
/// `backend_response_body()` as `application/json`.
async fn start_recording_backend() -> (u16, Recorded) {
    let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
    let recorded_for_handler = recorded.clone();
    let app = Router::new().fallback(
        move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
            let recorded = recorded_for_handler.clone();
            async move {
                let content_type = headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                recorded.lock().unwrap().push(RecordedRequest {
                    method,
                    path: uri.path().to_string(),
                    content_type,
                    body: body.to_vec(),
                });
                axum::Json(backend_response_body())
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

/// Write a JOLT spec to a temp file. The file must outlive the pipeline run.
fn write_spec(spec: &Value) -> NamedTempFile {
    let file = NamedTempFile::new().expect("create spec file");
    std::fs::write(file.path(), serde_json::to_vec_pretty(spec).unwrap()).expect("write spec");
    file
}

/// Build a config with one pipeline `under_test`. `endpoint_service` is the
/// inbound service, `middleware_blocks` holds the `[middleware.*]` tables and
/// `middleware_names` lists them in pipeline order.
fn pipeline_config(
    backend_port: u16,
    endpoint_service: &str,
    middleware_names: &[&str],
    middleware_blocks: &str,
) -> Config {
    let endpoint_options = if endpoint_service == "dicom_scp" {
        "local_aet = \"HARMONY_SCP\"\nenable_store = true"
    } else {
        "path_prefix = \"/api\""
    };
    let middleware_list = middleware_names
        .iter()
        .map(|n| format!("\"{}\"", n))
        .collect::<Vec<_>>()
        .join(", ");

    let toml = format!(
        r#"
[proxy]
id = "middleware-body-rewrite-test"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.tcp_config]
bind_address = "127.0.0.1"
bind_port = 11112

[targets.upstream_target]
connection.host = "127.0.0.1"
connection.port = {backend_port}
connection.protocol = "http"

[pipelines.under_test]
description = "middleware body rewrite"
networks = ["default"]
endpoints = ["inbound"]
backends = ["upstream"]
middleware = [{middleware_list}]

[endpoints.inbound]
service = "{endpoint_service}"
[endpoints.inbound.options]
{endpoint_options}

[backends.upstream]
service = "http"
target_ref = "upstream_target"

{middleware_blocks}

[services.http]
module = ""

[services.dicom_scp]
module = ""
"#
    );

    let mut config: Config = toml::from_str(&toml).expect("TOML parse error");
    harmony::config::resolution::resolve_references(&mut config)
        .expect("reference resolution failed");
    config.validate().expect("config validation failed");
    config
}

/// Build the envelope the HTTP endpoint produces for one inbound request.
async fn http_envelope(
    config: &Config,
    method: &str,
    path: &str,
    content_type: &str,
    body: Vec<u8>,
) -> (RequestEnvelope<Vec<u8>>, ProtocolCtx) {
    let mut meta = HashMap::new();
    meta.insert("path".to_string(), path.to_string());
    meta.insert("protocol".to_string(), "http".to_string());
    let ctx = ProtocolCtx {
        protocol: Protocol::Http,
        payload: body,
        meta,
        attrs: json!({
            "method": method,
            "uri": format!("/api{}", path),
            "headers": {
                "content-type": content_type,
                "content-length": "999",
            },
        }),
        command: None,
    };
    let envelope = build_envelope(config, &ctx).await;
    (envelope, ctx)
}

/// Build the envelope the DICOM SCP endpoint produces for a C-STORE whose
/// adapter body is `body`. The `path` meta entry gives the HTTP backend a
/// plain path, so these tests cover body handling only.
async fn dimse_store_envelope(
    config: &Config,
    body: &Value,
) -> (RequestEnvelope<Vec<u8>>, ProtocolCtx) {
    let mut meta = HashMap::new();
    meta.insert("protocol".to_string(), "dimse".to_string());
    meta.insert("path".to_string(), "/ingest".to_string());
    let ctx = ProtocolCtx {
        protocol: Protocol::Dimse,
        payload: serde_json::to_vec(body).unwrap(),
        meta,
        attrs: json!({}),
        command: Some(Command::new(Operation::Store)),
    };
    let envelope = build_envelope(config, &ctx).await;
    assert!(
        envelope.request_details.headers.is_empty(),
        "a DIMSE envelope has no HTTP headers"
    );
    (envelope, ctx)
}

async fn build_envelope(config: &Config, ctx: &ProtocolCtx) -> RequestEnvelope<Vec<u8>> {
    let endpoint = config.endpoints.get("inbound").expect("endpoint present");
    let service = endpoint
        .resolve_service()
        .expect("endpoint service resolves");
    let empty_options = HashMap::new();
    service
        .build_protocol_envelope(
            ctx.clone(),
            endpoint.options.as_ref().unwrap_or(&empty_options),
        )
        .await
        .expect("endpoint builds envelope")
}

/// Run the pipeline and return the single request the backend received plus
/// the response body bytes the client would get.
async fn run(
    config: &Config,
    envelope: RequestEnvelope<Vec<u8>>,
    ctx: &ProtocolCtx,
    recorded: &Recorded,
) -> (RecordedRequest, Vec<u8>) {
    let pipeline = config
        .pipelines
        .get("under_test")
        .expect("pipeline present")
        .clone();
    let response = PipelineExecutor::execute(envelope, "under_test", &pipeline, config, ctx)
        .await
        .expect("pipeline should reach the backend");
    assert_eq!(response.response_details.status, 200);

    let recorded = recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1, "backend should get exactly one request");
    (recorded[0].clone(), response.original_data)
}

#[tokio::test]
async fn transform_output_is_sent_to_http_backend_as_json() {
    let (port, recorded) = start_recording_backend().await;
    let spec = write_spec(&json!([{
        "operation": "shift",
        "spec": { "data": {
            "PatientID": "data.patient.id",
            "PatientName": "data.patient.name"
        } }
    }]));
    let blocks = format!(
        "[middleware.to_patient]\ntype = \"transform\"\n[middleware.to_patient.options]\nspec_path = \"{}\"\napply = \"left\"\n",
        spec.path().display()
    );
    let config = pipeline_config(port, "http", &["to_patient"], &blocks);

    let inbound = json!({ "PatientID": "123", "PatientName": "Doe", "Extra": "dropped" });
    let (envelope, ctx) = http_envelope(
        &config,
        "POST",
        "/patients",
        "application/json",
        serde_json::to_vec(&inbound).unwrap(),
    )
    .await;

    let (received, _) = run(&config, envelope, &ctx, &recorded).await;

    assert_eq!(received.method, Method::POST);
    assert_eq!(
        received.json_body(),
        json!({ "patient": { "id": "123", "name": "Doe" } })
    );
    assert_eq!(received.content_type.as_deref(), Some("application/json"));
}

#[tokio::test]
async fn transform_of_xml_request_sends_json_with_json_content_type() {
    let (port, recorded) = start_recording_backend().await;
    let spec = write_spec(&json!([{
        "operation": "shift",
        "spec": { "data": { "patient": {
            "id": "data.patientId",
            "name": "data.patientName"
        } } }
    }]));
    let blocks = format!(
        "[middleware.xml_to_json]\ntype = \"transform\"\n[middleware.xml_to_json.options]\nspec_path = \"{}\"\napply = \"left\"\n",
        spec.path().display()
    );
    let config = pipeline_config(port, "http", &["xml_to_json"], &blocks);

    let (envelope, ctx) = http_envelope(
        &config,
        "POST",
        "/patients",
        "application/xml",
        b"<patient><id>123</id><name>Doe</name></patient>".to_vec(),
    )
    .await;

    let (received, _) = run(&config, envelope, &ctx, &recorded).await;

    assert_eq!(
        received.json_body(),
        json!({ "patientId": "123", "patientName": "Doe" })
    );
    // The inbound application/xml must not describe the new JSON body.
    assert_eq!(received.content_type.as_deref(), Some("application/json"));
}

#[tokio::test]
async fn dicom_flatten_output_is_sent_to_http_backend() {
    let (port, recorded) = start_recording_backend().await;
    let blocks = "[middleware.flatten]\ntype = \"dicom_flatten\"\n";
    let config = pipeline_config(port, "dicom_scp", &["flatten"], blocks);

    // The body shape the DIMSE adapter builds for a C-STORE.
    let body = json!({
        "command": { "message_id": 1, "priority": "MEDIUM", "direction": "REQUEST" },
        "dataset": {
            "00100020": { "vr": "LO", "Value": ["PID156695"] },
            "00100010": { "vr": "PN", "Value": [{ "Alphabetic": "Doe^John" }] },
            "0020000D": { "vr": "UI", "Value": ["1.2.826.0.1.3680043.8.498.123"] }
        },
        "file": "/tmp/not-read-by-this-test.dcm"
    });
    let (envelope, ctx) = dimse_store_envelope(&config, &body).await;

    let (received, _) = run(&config, envelope, &ctx, &recorded).await;

    assert_eq!(received.path, "/ingest");
    let sent = received.json_body();
    assert_eq!(sent["dataset"]["00100020"], json!("PID156695"));
    assert_eq!(sent["dataset"]["00100010"], json!("Doe^John"));
    assert_eq!(
        sent["dataset"]["0020000D"],
        json!("1.2.826.0.1.3680043.8.498.123")
    );
    // Non-dataset fields of the DIMSE body are kept.
    assert_eq!(sent["file"], body["file"]);
    // A DIMSE request has no Content-Type of its own.
    assert_eq!(received.content_type.as_deref(), Some("application/json"));
}

#[tokio::test]
async fn passthru_only_pipeline_sends_binary_body_unchanged() {
    let (port, recorded) = start_recording_backend().await;
    let config = pipeline_config(port, "http", &["passthru"], "");

    // Not valid UTF-8 and not JSON. The bytes must reach the backend as is.
    let binary: Vec<u8> = vec![
        0x00, 0xFF, 0xFE, 0x80, 0x81, b'D', b'I', b'C', b'M', 0xC3, 0x28,
    ];
    assert!(std::str::from_utf8(&binary).is_err());
    let (envelope, ctx) = http_envelope(
        &config,
        "POST",
        "/upload",
        "application/octet-stream",
        binary.clone(),
    )
    .await;

    let (received, _) = run(&config, envelope, &ctx, &recorded).await;

    assert_eq!(received.body, binary);
    assert_eq!(
        received.content_type.as_deref(),
        Some("application/octet-stream")
    );
}

#[tokio::test]
async fn passthru_only_pipeline_sends_json_body_byte_for_byte() {
    let (port, recorded) = start_recording_backend().await;
    let config = pipeline_config(port, "http", &["passthru"], "");

    // Whitespace and key order would change if the body were re-serialised.
    let raw = b"{ \"z\": 1,\n  \"a\": [1, 2] }".to_vec();
    let (envelope, ctx) =
        http_envelope(&config, "POST", "/data", "application/json", raw.clone()).await;

    let (received, _) = run(&config, envelope, &ctx, &recorded).await;

    assert_eq!(received.body, raw);
}

#[tokio::test]
async fn dicom_to_dicomweb_store_sends_stow_multipart_to_http_backend() {
    let (port, recorded) = start_recording_backend().await;
    let blocks = "[middleware.to_dicomweb]\ntype = \"dicom_to_dicomweb\"\n";
    let config = pipeline_config(port, "dicom_scp", &["to_dicomweb"], blocks);

    // Stand-in for the Part 10 file the DIMSE adapter stages on disk.
    let part10: Vec<u8> = vec![0x00, 0xFF, 0x10, 0x80, b'D', b'I', b'C', b'M', 0xFE];
    let staged = NamedTempFile::new().expect("create staged file");
    std::fs::write(staged.path(), &part10).expect("write staged file");

    let body = json!({
        "command": { "message_id": 1, "priority": "MEDIUM", "direction": "REQUEST" },
        "dataset": { "00100020": { "vr": "LO", "Value": ["PID1"] } },
        "file": staged.path().to_string_lossy()
    });
    let (envelope, ctx) = dimse_store_envelope(&config, &body).await;

    let (received, _) = run(&config, envelope, &ctx, &recorded).await;

    assert_eq!(received.method, Method::POST);
    assert_eq!(received.path, "/studies");
    let content_type = received.content_type.expect("content-type sent");
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .expect("multipart boundary in content-type");
    assert!(content_type.starts_with("multipart/related; type=\"application/dicom\""));

    let mut expected =
        format!("--{}\r\nContent-Type: application/dicom\r\n\r\n", boundary).into_bytes();
    expected.extend_from_slice(&part10);
    expected.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
    assert_eq!(received.body, expected);
}

#[tokio::test]
async fn right_side_transform_output_reaches_the_client() {
    let (port, recorded) = start_recording_backend().await;
    let spec = write_spec(&json!([{
        "operation": "shift",
        "spec": { "data": { "patient": { "id": "data.subjectId" } } }
    }]));
    let blocks = format!(
        "[middleware.reshape_response]\ntype = \"transform\"\n[middleware.reshape_response.options]\nspec_path = \"{}\"\napply = \"right\"\n",
        spec.path().display()
    );
    let config = pipeline_config(port, "http", &["reshape_response"], &blocks);

    let inbound = br#"{"q":"x"}"#.to_vec();
    let (envelope, ctx) = http_envelope(
        &config,
        "POST",
        "/lookup",
        "application/json",
        inbound.clone(),
    )
    .await;

    let (received, client_body) = run(&config, envelope, &ctx, &recorded).await;

    // apply = "right" leaves the request body alone.
    assert_eq!(received.body, inbound);
    let client_json: Value = serde_json::from_slice(&client_body).expect("client body is JSON");
    assert_eq!(client_json, json!({ "subjectId": "123" }));
}
