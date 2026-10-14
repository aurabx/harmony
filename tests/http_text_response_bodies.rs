//! Integration tests for non-JSON text responses from an HTTP backend.
//!
//! A backend that answers with `text/html`, `text/plain` or `application/xml`
//! must reach the client with the same body and Content-Type, with or without
//! middleware on the response side. A body that claims to be JSON but is not
//! must arrive unchanged too.

use axum::body::Body;
use axum::http::{Method, Response, StatusCode, Uri};
use axum::Router;
use harmony::config::config::Config;
use harmony::models::envelope::envelope::ResponseEnvelope;
use harmony::models::protocol::{Protocol, ProtocolCtx};
use harmony::pipeline::PipelineExecutor;
use std::collections::HashMap;
use tempfile::NamedTempFile;
use tokio::net::TcpListener;

const HTML_BODY: &str = "<html><body>hello html</body></html>";
const PLAIN_BODY: &str = "hello plain";
const XML_BODY: &str = "<a><b>hello xml</b></a>";
const LATIN1_BODY: &[u8] = &[0x68, 0xe9, 0x6c, 0x6c, 0xf6];
const ERROR_BODY: &str = "<h1>upstream failure</h1>";
const BAD_JSON_BODY: &str = "<html>502 Bad Gateway</html>";
const BOM_JSON_BODY: &[u8] = b"\xef\xbb\xbf{\"bom\":true}";

/// What the test backend returns for one path.
struct Canned {
    status: u16,
    content_type: Option<&'static str>,
    body: &'static [u8],
}

fn canned_response(path: &str) -> Canned {
    let (status, content_type, body): (u16, Option<&'static str>, &'static [u8]) = match path {
        "/html" => (200, Some("text/html"), HTML_BODY.as_bytes()),
        "/plain" => (
            200,
            Some("text/plain; charset=utf-8"),
            PLAIN_BODY.as_bytes(),
        ),
        "/xml" => (200, Some("application/xml"), XML_BODY.as_bytes()),
        "/upper" => (200, Some("TEXT/HTML"), HTML_BODY.as_bytes()),
        "/latin1" => (200, Some("text/plain"), LATIN1_BODY),
        "/no-content-type" => (200, None, PLAIN_BODY.as_bytes()),
        "/not-found" => (404, Some("text/html"), ERROR_BODY.as_bytes()),
        "/bad-gateway" => (502, Some("text/html"), ERROR_BODY.as_bytes()),
        "/bad-json" => (200, Some("application/json"), BAD_JSON_BODY.as_bytes()),
        "/bom-json" => (200, Some("application/json"), BOM_JSON_BODY),
        "/not-modified" => (304, Some("text/html"), b""),
        "/no-content" => (204, Some("text/html"), b""),
        "/json" => (200, Some("application/json"), br#"{"hello":"json"}"#),
        other => panic!("test backend has no canned response for {other}"),
    };
    Canned {
        status,
        content_type,
        body,
    }
}

/// Start a backend that answers each path with its canned response. A HEAD
/// request gets the same status and headers and no body.
async fn start_backend() -> (u16, tokio::task::JoinHandle<()>) {
    let app = Router::new().fallback(|method: Method, uri: Uri| async move {
        let canned = canned_response(uri.path());
        let mut builder = Response::builder().status(canned.status);
        if let Some(content_type) = canned.content_type {
            builder = builder.header("content-type", content_type);
        }
        let body = if method == Method::HEAD || canned.body.is_empty() {
            Body::empty()
        } else {
            Body::from(canned.body)
        };
        builder.body(body).unwrap()
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, handle)
}

fn http_to_http_config(
    backend_port: u16,
    middleware_names: &[&str],
    middleware_blocks: &str,
) -> Config {
    let middleware_list = middleware_names
        .iter()
        .map(|n| format!("\"{}\"", n))
        .collect::<Vec<_>>()
        .join(", ");
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
description = "HTTP passthrough"
networks = ["default"]
endpoints = ["front"]
backends = ["upstream"]
middleware = [{middleware_list}]

[endpoints.front]
service = "http"
[endpoints.front.options]
path_prefix = ""

[backends.upstream]
service = "http"
target_ref = "upstream_target"

{middleware_blocks}

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

/// Send `method path` through the pipeline in `config`. Returns the executor's
/// response and the context the endpoint needs to convert it.
async fn run_pipeline(
    config: &Config,
    method: &str,
    path: &str,
) -> (ResponseEnvelope<Vec<u8>>, ProtocolCtx) {
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
            "method": method,
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

    let response = PipelineExecutor::execute(envelope, "text_passthrough", &pipeline, config, &ctx)
        .await
        .expect("pipeline should reach the HTTP backend");
    (response, ctx)
}

fn content_type_of(response: &ResponseEnvelope<Vec<u8>>) -> Option<String> {
    response
        .response_details
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone())
}

/// Send a GET for `path` through a pipeline with no middleware.
async fn get_through_pipeline(path: &str) -> (u16, Option<String>, Vec<u8>) {
    let (port, _handle) = start_backend().await;
    let config = http_to_http_config(port, &[], "");
    let (response, _ctx) = run_pipeline(&config, "GET", path).await;
    (
        response.response_details.status,
        content_type_of(&response),
        response.original_data.clone(),
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
async fn upper_case_content_type_is_matched_case_insensitively() {
    let (status, content_type, body) = get_through_pipeline("/upper").await;
    assert_eq!(status, 200);
    assert_eq!(content_type.as_deref(), Some("TEXT/HTML"));
    assert_eq!(body, HTML_BODY.as_bytes());
}

#[tokio::test]
async fn non_utf8_text_body_reaches_the_client_unchanged() {
    let (status, _content_type, body) = get_through_pipeline("/latin1").await;
    assert_eq!(status, 200);
    assert_eq!(body, LATIN1_BODY);
}

#[tokio::test]
async fn text_body_without_a_content_type_reaches_the_client() {
    let (status, _content_type, body) = get_through_pipeline("/no-content-type").await;
    assert_eq!(status, 200);
    assert_eq!(body, PLAIN_BODY.as_bytes());
}

#[tokio::test]
async fn text_error_pages_keep_their_status_and_body() {
    for (path, expected_status) in [("/not-found", 404u16), ("/bad-gateway", 502)] {
        let (status, content_type, body) = get_through_pipeline(path).await;
        assert_eq!(status, expected_status, "{path}");
        assert_eq!(content_type.as_deref(), Some("text/html"), "{path}");
        assert_eq!(body, ERROR_BODY.as_bytes(), "{path}");
    }
}

#[tokio::test]
async fn bodies_that_claim_to_be_json_but_are_not_reach_the_client() {
    let (status, content_type, body) = get_through_pipeline("/bad-json").await;
    assert_eq!(status, 200);
    assert_eq!(content_type.as_deref(), Some("application/json"));
    assert_eq!(body, BAD_JSON_BODY.as_bytes());

    let (_, _, body) = get_through_pipeline("/bom-json").await;
    assert_eq!(body, BOM_JSON_BODY);
}

#[tokio::test]
async fn empty_responses_stay_empty() {
    for (path, expected_status) in [("/not-modified", 304u16), ("/no-content", 204)] {
        let (status, content_type, body) = get_through_pipeline(path).await;
        assert_eq!(status, expected_status, "{path}");
        assert_eq!(content_type.as_deref(), Some("text/html"), "{path}");
        assert!(body.is_empty(), "{path}");
    }
}

#[tokio::test]
async fn head_request_gets_the_headers_and_no_body() {
    let (port, _handle) = start_backend().await;
    let config = http_to_http_config(port, &[], "");
    let (response, _ctx) = run_pipeline(&config, "HEAD", "/html").await;
    assert_eq!(response.response_details.status, 200);
    assert_eq!(content_type_of(&response).as_deref(), Some("text/html"));
    assert!(response.original_data.is_empty());
}

#[tokio::test]
async fn json_body_is_unchanged() {
    let (status, content_type, body) = get_through_pipeline("/json").await;
    assert_eq!(status, 200);
    assert_eq!(content_type.as_deref(), Some("application/json"));
    assert_eq!(body, br#"{"hello":"json"}"#);
}

/// Response-side middleware only works on JSON, so it must leave a text body
/// intact.
#[tokio::test]
async fn text_body_survives_response_side_middleware() {
    let spec = NamedTempFile::new().expect("create spec file");
    std::fs::write(spec.path(), br#"[{"operation":"shift","spec":{"*":"&"}}]"#)
        .expect("write spec");
    let blocks = format!(
        "[middleware.extract]\ntype = \"json_extractor\"\n\
         [middleware.passthrough]\ntype = \"passthru\"\n\
         [middleware.reshape]\ntype = \"transform\"\n\
         [middleware.reshape.options]\nspec_path = \"{}\"\napply = \"right\"\n",
        spec.path().display()
    );

    let (port, _handle) = start_backend().await;
    let config = http_to_http_config(port, &["extract", "passthrough", "reshape"], &blocks);

    for (path, expected) in [
        ("/html", HTML_BODY.as_bytes()),
        ("/xml", XML_BODY.as_bytes()),
        ("/plain", PLAIN_BODY.as_bytes()),
    ] {
        let (response, _ctx) = run_pipeline(&config, "GET", path).await;
        assert_eq!(response.response_details.status, 200, "{path}");
        assert_eq!(response.original_data, expected, "{path}");
    }

    // Control: the chain did run. `passthru` marks JSON responses with `mw_right`.
    let (response, _ctx) = run_pipeline(&config, "GET", "/json").await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.original_data)
            .expect("JSON stays JSON"),
        serde_json::json!({"hello": "json", "mw_right": true})
    );
}

/// The last hop: the endpoint turns the response envelope into the HTTP
/// response the client reads.
#[tokio::test]
async fn http_endpoint_response_carries_the_text_body() {
    let (port, _handle) = start_backend().await;
    let config = http_to_http_config(port, &[], "");
    let endpoint = config.endpoints.get("front").expect("endpoint present");
    let service = endpoint.resolve_service().expect("http resolves");
    let empty_options = HashMap::new();

    for (path, content_type, expected) in [
        ("/html", "text/html", HTML_BODY.as_bytes()),
        ("/xml", "application/xml", XML_BODY.as_bytes()),
        ("/plain", "text/plain; charset=utf-8", PLAIN_BODY.as_bytes()),
    ] {
        let (response, _ctx) = run_pipeline(&config, "GET", path).await;
        let http_response = service
            .endpoint_outgoing_response(
                response,
                endpoint.options.as_ref().unwrap_or(&empty_options),
            )
            .await
            .expect("endpoint builds the HTTP response");

        assert_eq!(http_response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            http_response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some(content_type),
            "{path}"
        );
        let body = axum::body::to_bytes(http_response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        assert_eq!(body.as_ref(), expected, "{path}");
    }
}
