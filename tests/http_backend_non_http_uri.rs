//! Integration tests for a DIMSE endpoint feeding a generic HTTP backend.
//!
//! A DICOM SCP endpoint builds its request envelope with a protocol URI such
//! as `dimse://scp/store`. That URI must never reach the HTTP backend URL.
//! The backend path comes only from the target (`base_path`) and the
//! backend's `path_prefix` option.

use axum::http::{Method, Uri};
use axum::Router;
use harmony::config::config::Config;
use harmony::models::envelope::command::{Command, Operation};
use harmony::models::protocol::{Protocol, ProtocolCtx};
use harmony::pipeline::PipelineExecutor;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

type RecordedRequests = Arc<Mutex<Vec<(Method, String)>>>;

/// Start an upstream server that records the method and path-and-query of
/// every request it receives. It uses a fallback so the root path `/` is
/// recorded as well.
async fn start_recording_backend() -> (u16, RecordedRequests, tokio::task::JoinHandle<()>) {
    let recorded: RecordedRequests = Arc::new(Mutex::new(Vec::new()));
    let recorded_for_handler = recorded.clone();
    let app = Router::new().fallback(move |method: Method, uri: Uri| {
        let recorded = recorded_for_handler.clone();
        async move {
            let path = uri
                .path_and_query()
                .map(|pq| pq.to_string())
                .unwrap_or_else(|| uri.path().to_string());
            recorded.lock().unwrap().push((method, path));
            axum::Json(serde_json::json!({ "backend": "reached" }))
        }
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (port, recorded, handle)
}

/// Pipeline with a DICOM SCP endpoint and a plain HTTP backend. `base_path`
/// and `path_prefix` are optional so each test can pick its URL shape.
fn dimse_to_http_config(
    backend_port: u16,
    base_path: Option<&str>,
    path_prefix: Option<&str>,
) -> Config {
    let base_path_line = base_path
        .map(|p| format!("connection.base_path = \"{}\"", p))
        .unwrap_or_default();
    let path_prefix_block = path_prefix
        .map(|p| format!("[backends.upstream.options]\npath_prefix = \"{}\"", p))
        .unwrap_or_default();

    let toml = format!(
        r#"
[proxy]
id = "http-backend-non-http-uri-test"

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
{base_path_line}

[pipelines.dimse_to_http]
description = "DICOM SCP to generic HTTP backend"
networks = ["default"]
endpoints = ["scp"]
backends = ["upstream"]

[endpoints.scp]
service = "dicom_scp"
[endpoints.scp.options]
local_aet = "HARMONY_SCP"
enable_store = true

[backends.upstream]
service = "http"
target_ref = "upstream_target"
{path_prefix_block}

[services.dicom_scp]
module = ""

[services.http]
module = ""
"#
    );

    let mut config: Config = toml::from_str(&toml).expect("TOML parse error");
    harmony::config::resolution::resolve_references(&mut config)
        .expect("reference resolution failed");
    config.validate().expect("config validation failed");
    config
}

/// Run one C-STORE-shaped request through the pipeline and return what the
/// HTTP backend received.
async fn send_cstore_through_http_backend(
    base_path: Option<&str>,
    path_prefix: Option<&str>,
) -> Vec<(Method, String)> {
    let (port, recorded, _handle) = start_recording_backend().await;
    let config = dimse_to_http_config(port, base_path, path_prefix);
    let pipeline = config
        .pipelines
        .get("dimse_to_http")
        .expect("pipeline present")
        .clone();

    let ctx = ProtocolCtx {
        protocol: Protocol::Dimse,
        payload: b"not-a-real-dicom-file".to_vec(),
        meta: HashMap::new(),
        attrs: serde_json::json!({}),
        command: Some(Command::new(Operation::Store)),
    };

    // Build the envelope the same way the DIMSE adapter does, so the URI is
    // the real `dimse://scp/store` value from the SCP endpoint.
    let endpoint = config.endpoints.get("scp").expect("endpoint present");
    let service = endpoint.resolve_service().expect("dicom_scp resolves");
    let empty_options = HashMap::new();
    let envelope = service
        .build_protocol_envelope(
            ctx.clone(),
            endpoint.options.as_ref().unwrap_or(&empty_options),
        )
        .await
        .expect("SCP builds envelope");
    assert_eq!(envelope.request_details.uri, "dimse://scp/store");

    let response = PipelineExecutor::execute(envelope, "dimse_to_http", &pipeline, &config, &ctx)
        .await
        .expect("pipeline should reach the HTTP backend");
    assert_eq!(response.response_details.status, 200);

    let recorded = recorded.lock().unwrap().clone();
    recorded
}

#[tokio::test]
async fn dimse_uri_is_replaced_by_base_path_and_path_prefix() {
    let recorded = send_cstore_through_http_backend(Some("/base"), Some("/ingest")).await;
    assert_eq!(recorded, vec![(Method::POST, "/base/ingest".to_string())]);
}

#[tokio::test]
async fn dimse_uri_with_path_prefix_only() {
    let recorded = send_cstore_through_http_backend(None, Some("/ingest/")).await;
    assert_eq!(recorded, vec![(Method::POST, "/ingest".to_string())]);
}

#[tokio::test]
async fn dimse_uri_with_base_path_only() {
    let recorded = send_cstore_through_http_backend(Some("/base"), None).await;
    assert_eq!(recorded, vec![(Method::POST, "/base".to_string())]);
}

#[tokio::test]
async fn dimse_uri_without_base_path_or_prefix_targets_root() {
    let recorded = send_cstore_through_http_backend(None, None).await;
    assert_eq!(recorded, vec![(Method::POST, "/".to_string())]);
}
