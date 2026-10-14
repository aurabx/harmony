//! Integration tests for non-JSON text responses from an HTTP backend.
//!
//! A backend that answers with `text/html`, `text/plain` or `application/xml`
//! must reach the client with the same body and Content-Type, even when the
//! pipeline has no middleware.

use axum::http::header::CONTENT_TYPE;
use axum::routing::get;
use axum::Router;
use harmony::config::config::Config;
use harmony::models::protocol::{Protocol, ProtocolCtx};
use harmony::pipeline::PipelineExecutor;
use std::collections::HashMap;
use tokio::net::TcpListener;

const HTML_BODY: &str = "<html><body>hello html</body></html>";
const PLAIN_BODY: &str = "hello plain";
const XML_BODY: &str = "<a><b>hello xml</b></a>";
const LATIN1_BODY: &[u8] = &[0x68, 0xe9, 0x6c, 0x6c, 0xf6];

async fn start_text_backend() -> (u16, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route(
            "/html",
            get(|| async { ([(CONTENT_TYPE, "text/html")], HTML_BODY) }),
        )
        .route(
            "/plain",
            get(|| async { ([(CONTENT_TYPE, "text/plain; charset=utf-8")], PLAIN_BODY) }),
        )
        .route(
            "/xml",
            get(|| async { ([(CONTENT_TYPE, "application/xml")], XML_BODY) }),
        )
        .route(
            "/latin1",
            get(|| async { ([(CONTENT_TYPE, "text/plain")], LATIN1_BODY) }),
        )
        .route(
            "/json",
            get(|| async { ([(CONTENT_TYPE, "application/json")], r#"{"hello":"json"}"#) }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, handle)
}

fn http_to_http_config(backend_port: u16) -> Config {
    let toml = format!(
        r#"
[proxy]
id = "http-text-response-bodies-test"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 18080

[targets.upstream_target]
connection.host = "127.0.0.1"
connection.port = {backend_port}
connection.protocol = "http"

[pipelines.text_passthrough]
description = "HTTP passthrough with no middleware"
networks = ["default"]
endpoints = ["front"]
backends = ["upstream"]

[endpoints.front]
service = "http"
[endpoints.front.options]
path_prefix = ""

[backends.upstream]
service = "http"
target_ref = "upstream_target"

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

/// Send a GET for `path` through the pipeline and return the response status,
/// Content-Type header and body bytes.
async fn get_through_pipeline(path: &str) -> (u16, Option<String>, Vec<u8>) {
    let (port, _handle) = start_text_backend().await;
    let config = http_to_http_config(port);
    let pipeline = config
        .pipelines
        .get("text_passthrough")
        .expect("pipeline present")
        .clone();

    let ctx = ProtocolCtx {
        protocol: Protocol::Http,
        payload: Vec::new(),
        meta: HashMap::new(),
        attrs: serde_json::json!({
            "method": "GET",
            "uri": path,
            "path": path,
            "headers": {},
            "cookies": {},
            "query_params": {},
        }),
        command: None,
    };

    let endpoint = config.endpoints.get("front").expect("endpoint present");
    let service = endpoint.resolve_service().expect("http resolves");
    let empty_options = HashMap::new();
    let envelope = service
        .build_protocol_envelope(
            ctx.clone(),
            endpoint.options.as_ref().unwrap_or(&empty_options),
        )
        .await
        .expect("endpoint builds envelope");

    let response =
        PipelineExecutor::execute(envelope, "text_passthrough", &pipeline, &config, &ctx)
            .await
            .expect("pipeline should reach the HTTP backend");

    let content_type = response
        .response_details
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone());
    (
        response.response_details.status,
        content_type,
        response.original_data,
    )
}

#[tokio::test]
async fn text_html_body_reaches_the_client() {
    let (status, content_type, body) = get_through_pipeline("/html").await;
    assert_eq!(status, 200);
    assert_eq!(content_type.as_deref(), Some("text/html"));
    assert_eq!(body, HTML_BODY.as_bytes());
}

#[tokio::test]
async fn text_plain_body_reaches_the_client() {
    let (status, content_type, body) = get_through_pipeline("/plain").await;
    assert_eq!(status, 200);
    assert_eq!(content_type.as_deref(), Some("text/plain; charset=utf-8"));
    assert_eq!(body, PLAIN_BODY.as_bytes());
}

#[tokio::test]
async fn application_xml_body_reaches_the_client() {
    let (status, content_type, body) = get_through_pipeline("/xml").await;
    assert_eq!(status, 200);
    assert_eq!(content_type.as_deref(), Some("application/xml"));
    assert_eq!(body, XML_BODY.as_bytes());
}

#[tokio::test]
async fn non_utf8_text_body_reaches_the_client_unchanged() {
    let (status, _content_type, body) = get_through_pipeline("/latin1").await;
    assert_eq!(status, 200);
    assert_eq!(body, LATIN1_BODY);
}

#[tokio::test]
async fn json_body_is_unchanged() {
    let (status, content_type, body) = get_through_pipeline("/json").await;
    assert_eq!(status, 200);
    assert_eq!(content_type.as_deref(), Some("application/json"));
    assert_eq!(body, br#"{"hello":"json"}"#);
}
