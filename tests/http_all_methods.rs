//! Every standard HTTP method reaches the pipeline of an `http` endpoint.
//!
//! The router used to register only GET, POST, PUT and DELETE for `http`
//! endpoints, so PATCH, HEAD and OPTIONS got 405 before any policy or backend
//! ran. A recording upstream shows what the HTTP backend actually forwarded.
//! Linear: AURA-2440.

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri};
use axum::routing::any;
use axum::Router;
use harmony::config::config::Config;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tower::ServiceExt;

/// One request as the upstream saw it.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    body: Vec<u8>,
}

struct Upstream {
    base_url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _handle: tokio::task::JoinHandle<()>,
}

impl Upstream {
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

async fn start_recording_upstream() -> Upstream {
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let handler = move |method: Method, uri: Uri, _headers: HeaderMap, body: Bytes| {
        let sink = sink.clone();
        async move {
            sink.lock().unwrap().push(Seen {
                method: method.to_string(),
                path: uri.path().to_string(),
                body: body.to_vec(),
            });
            (
                [
                    ("content-type", "application/json"),
                    ("x-upstream", "recorded"),
                ],
                "{\"ok\":true}",
            )
        }
    };
    let app = Router::new()
        .route("/", any(handler.clone()))
        .route("/{*path}", any(handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Upstream {
        base_url,
        seen,
        _handle: handle,
    }
}

/// `/proxy/*` on an `http` endpoint, forwarded by an `http` backend.
/// `middleware` and `extra` let a test add a policy.
fn proxy_config(base_url: &str, middleware: &str, extra: &str) -> String {
    format!(
        r#"
        [proxy]
        id = "http-all-methods-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["upstream"]
        middleware = [{middleware}]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/proxy"

        [backends.upstream]
        service = "http"
        [backends.upstream.options]
        base_url = "{base_url}"

        [services.http]
        module = ""

        {extra}
        "#
    )
}

/// A policies middleware whose only rule allows the given methods.
fn method_policy(methods: &str) -> String {
    format!(
        r#"
        [middleware.method_policy]
        type = "policies"
        [middleware.method_policy.options]
        policies = ["methods"]

        [policies.methods]
        id = "methods"
        enabled = true
        rules = ["method_rule"]

        [rules.method_rule]
        id = "method_rule"
        type = "method"
        weight = 100
        enabled = true
        [rules.method_rule.options]
        methods = [{methods}]
        mode = "allow"

        [middleware_types.policies]
        module = ""
        "#
    )
}

async fn build_app(toml: &str) -> Router {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate().expect("valid config");
    harmony::router::build_network_router(Arc::new(config), "default").await
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: &'static str,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if !body.is_empty() {
        builder = builder.header("content-type", "application/json");
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .expect("router response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, headers, bytes.to_vec())
}

#[tokio::test]
async fn patch_reaches_the_upstream_with_its_body() {
    let upstream = start_recording_upstream().await;
    let app = build_app(&proxy_config(&upstream.base_url, "", "")).await;

    let (status, _, body) = send(&app, "PATCH", "/proxy/items/42", r#"{"op":"replace"}"#).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let seen = upstream.requests();
    assert_eq!(seen.len(), 1, "upstream requests: {:?}", seen);
    assert_eq!(seen[0].method, "PATCH");
    assert_eq!(seen[0].path, "/items/42");
    assert_eq!(seen[0].body, br#"{"op":"replace"}"#.to_vec());
}

#[tokio::test]
async fn head_returns_upstream_headers_without_a_body() {
    let upstream = start_recording_upstream().await;
    let app = build_app(&proxy_config(&upstream.base_url, "", "")).await;

    let (status, headers, body) = send(&app, "HEAD", "/proxy/items/42", "").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("x-upstream").and_then(|v| v.to_str().ok()),
        Some("recorded")
    );
    assert!(body.is_empty(), "HEAD response had a body: {:?}", body);
    let seen = upstream.requests();
    assert_eq!(seen.len(), 1, "upstream requests: {:?}", seen);
    assert_eq!(seen[0].method, "HEAD");
    assert_eq!(seen[0].path, "/items/42");
}

#[tokio::test]
async fn options_reaches_the_pipeline_and_the_upstream() {
    let upstream = start_recording_upstream().await;
    let app = build_app(&proxy_config(&upstream.base_url, "", "")).await;

    let (status, _, body) = send(&app, "OPTIONS", "/proxy/items", "").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let seen = upstream.requests();
    assert_eq!(seen.len(), 1, "upstream requests: {:?}", seen);
    assert_eq!(seen[0].method, "OPTIONS");
    assert_eq!(seen[0].path, "/items");
}

#[tokio::test]
async fn exact_prefix_path_accepts_patch() {
    let upstream = start_recording_upstream().await;
    let app = build_app(&proxy_config(&upstream.base_url, "", "")).await;

    let (status, _, _) = send(&app, "PATCH", "/proxy", r#"{"a":1}"#).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream.requests().len(), 1);
}

#[tokio::test]
async fn trace_is_not_routed() {
    let upstream = start_recording_upstream().await;
    let app = build_app(&proxy_config(&upstream.base_url, "", "")).await;

    let (status, _, _) = send(&app, "TRACE", "/proxy/items", "").await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(upstream.requests().is_empty());
}

#[tokio::test]
async fn policy_method_rule_admits_patch_when_listed() {
    let upstream = start_recording_upstream().await;
    let toml = proxy_config(
        &upstream.base_url,
        "\"method_policy\"",
        &method_policy("\"GET\", \"PATCH\""),
    );
    let app = build_app(&toml).await;

    let (status, _, _) = send(&app, "PATCH", "/proxy/items/42", r#"{"op":"replace"}"#).await;

    assert_eq!(status, StatusCode::OK);
    let seen = upstream.requests();
    assert_eq!(seen.len(), 1, "upstream requests: {:?}", seen);
    assert_eq!(seen[0].method, "PATCH");
}

#[tokio::test]
async fn policy_method_rule_denies_patch_with_405() {
    let upstream = start_recording_upstream().await;
    let toml = proxy_config(
        &upstream.base_url,
        "\"method_policy\"",
        &method_policy("\"GET\""),
    );
    let app = build_app(&toml).await;

    let (status, _, _) = send(&app, "PATCH", "/proxy/items/42", r#"{"op":"replace"}"#).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(upstream.requests().is_empty());

    // The same pipeline still admits GET, so the 405 came from the policy.
    let (status, _, _) = send(&app, "GET", "/proxy/items/42", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream.requests().len(), 1);
}

#[tokio::test]
async fn head_drops_a_body_the_pipeline_produced() {
    // The echo backend answers with a body for every method, so only the
    // router can keep it out of a HEAD response.
    let toml = r#"
        [proxy]
        id = "http-all-methods-echo"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo"]
        middleware = []

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/proxy"

        [backends.echo]
        service = "echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""
    "#;
    let app = build_app(toml).await;

    let (get_status, _, get_body) = send(&app, "GET", "/proxy/items", "").await;
    assert_eq!(get_status, StatusCode::OK);
    assert!(
        !get_body.is_empty(),
        "echo backend should answer GET with a body"
    );

    let (status, _, body) = send(&app, "HEAD", "/proxy/items", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.is_empty(),
        "HEAD response had a body: {:?}",
        String::from_utf8_lossy(&body)
    );
}

#[test]
fn generic_http_endpoints_route_every_standard_method() {
    let standard = [
        Method::GET,
        Method::HEAD,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];
    for service_name in ["http", "http_streaming", "echo"] {
        let service = harmony::models::services::services::resolve_service(service_name)
            .unwrap_or_else(|e| panic!("resolve {}: {}", service_name, e));
        let mut options = std::collections::HashMap::new();
        options.insert("path_prefix".to_string(), serde_json::json!("/api"));
        let routes = service.build_router(&options);
        assert!(!routes.is_empty(), "{} built no routes", service_name);
        for route in &routes {
            for method in &standard {
                assert!(
                    route.methods.contains(method),
                    "{}: {} missing on {}",
                    service_name,
                    method,
                    route.path
                );
            }
            assert!(!route.methods.contains(&Method::TRACE));
            assert!(!route.methods.contains(&Method::CONNECT));
        }
    }
}
