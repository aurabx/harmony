//! Which client headers reach the upstream through a backend.
//!
//! A recording upstream captures every header it receives, including repeated
//! ones, so the tests can prove that the upstream gets exactly one
//! `Authorization` header and never the client's credential by default.
//! Linear: AURA-2433, AURA-2411, AURA-2427.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use futures_util::StreamExt;
use harmony::config::config::Config;
use harmony::models::envelope::envelope::RequestEnvelope;
use harmony::models::services::streaming::StreamingBackend;
use harmony::models::services::types::http_streaming_backend::HttpStreamingBackend;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tower::ServiceExt;

const CLIENT_AUTH: &str = "Basic Y2xpZW50OmNsaWVudC1wYXNzd29yZA==";
const TARGET_TOKEN: &str = "target-token-123";
const MESH_SECRET: &str = "backend-header-forwarding-mesh-secret";

/// Every header the upstream received, lowercased name first, in arrival order.
type Recorded = Arc<Mutex<Vec<(String, String)>>>;

struct Upstream {
    base_url: String,
    recorded: Recorded,
    _handle: tokio::task::JoinHandle<()>,
}

impl Upstream {
    fn values(&self, name: &str) -> Vec<String> {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn has(&self, name: &str) -> bool {
        !self.values(name).is_empty()
    }

    fn authority(&self) -> &str {
        self.base_url.trim_start_matches("http://")
    }
}

async fn start_recording_upstream() -> Upstream {
    let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
    let sink = recorded.clone();
    let app = Router::new().route(
        "/{*path}",
        any(move |headers: axum::http::HeaderMap| {
            let sink = sink.clone();
            async move {
                let mut all = sink.lock().unwrap();
                all.clear();
                for (name, value) in headers.iter() {
                    all.push((
                        name.as_str().to_string(),
                        value.to_str().unwrap_or("<non-utf8>").to_string(),
                    ));
                }
                // NDJSON so the same upstream also serves the streaming backend.
                (
                    [("content-type", "application/x-ndjson")],
                    "{\"ok\":true}\n",
                )
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Upstream {
        base_url,
        recorded,
        _handle: handle,
    }
}

fn load_config(toml: &str) -> Config {
    let mut config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate().expect("valid config");
    harmony::config::resolution::resolve_references(&mut config).expect("resolve references");
    config
}

/// A pipeline `/proxy/*` -> backend `upstream`. `backend_extra` goes in the
/// backend table, `backend_options` in its options table, `extra` at the end.
fn proxy_config(
    service: &str,
    base_url: &str,
    backend_extra: &str,
    backend_options: &str,
    extra: &str,
) -> String {
    format!(
        r#"
        [proxy]
        id = "backend-header-forwarding-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [authentications.target-bearer]
        id = "target-bearer"
        method = "bearer"
        [authentications.target-bearer.options]
        token = "{token}"

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["upstream"]
        middleware = []

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/proxy"

        [backends.upstream]
        service = "{service}"
        {backend_extra}
        [backends.upstream.options]
        base_url = "{base_url}"
        {backend_options}

        [services.http]
        module = ""

        [services.fhir]
        module = ""

        {extra}
    "#,
        token = TARGET_TOKEN,
    )
}

async fn send(config: Config, headers: &[(&str, &str)]) -> StatusCode {
    let config = Arc::new(config);
    harmony::globals::set_config(config.clone());
    let app = harmony::router::build_network_router(config, "default").await;
    let mut request = Request::builder().uri("/proxy/resource").method("GET");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .expect("router handled request");
    response.status()
}

#[tokio::test]
#[serial_test::serial]
async fn inbound_authorization_is_not_forwarded_by_default() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config("http", &upstream.base_url, "", "", ""));

    let status = send(config, &[("authorization", CLIENT_AUTH)]).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        !upstream.has("authorization"),
        "client Authorization reached the upstream: {:?}",
        upstream.values("authorization")
    );
}

#[tokio::test]
#[serial_test::serial]
async fn target_auth_is_sent_exactly_once_and_client_auth_is_dropped() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        "http",
        &upstream.base_url,
        r#"authentication = "target-bearer""#,
        "",
        "",
    ));

    send(config, &[("authorization", CLIENT_AUTH)]).await;

    assert_eq!(
        upstream.values("authorization"),
        vec![format!("Bearer {}", TARGET_TOKEN)]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn fhir_target_auth_is_sent_exactly_once_and_client_auth_is_dropped() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        "fhir",
        &upstream.base_url,
        r#"authentication = "target-bearer""#,
        "",
        "",
    ));

    send(config, &[("authorization", CLIENT_AUTH)]).await;

    assert_eq!(
        upstream.values("authorization"),
        vec![format!("Bearer {}", TARGET_TOKEN)]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn forward_authorization_passes_client_auth_when_target_has_none() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        "http",
        &upstream.base_url,
        "",
        "forward_authorization = true",
        "",
    ));

    send(config, &[("authorization", CLIENT_AUTH)]).await;

    assert_eq!(
        upstream.values("authorization"),
        vec![CLIENT_AUTH.to_string()]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn forward_authorization_with_target_auth_sends_only_target_auth() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        "http",
        &upstream.base_url,
        r#"authentication = "target-bearer""#,
        "forward_authorization = true",
        "",
    ));

    send(config, &[("authorization", CLIENT_AUTH)]).await;

    assert_eq!(
        upstream.values("authorization"),
        vec![format!("Bearer {}", TARGET_TOKEN)]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn host_and_hop_by_hop_headers_are_not_forwarded_but_ordinary_headers_are() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config("http", &upstream.base_url, "", "", ""));

    let status = send(
        config,
        &[
            ("host", "client-facing.example.com"),
            ("connection", "keep-alive, x-hop-custom"),
            ("keep-alive", "timeout=5"),
            ("proxy-authorization", "Basic cHJveHk6cHJveHk="),
            ("te", "trailers"),
            ("trailer", "x-checksum"),
            ("upgrade", "h2c"),
            ("x-hop-custom", "named-in-connection"),
            ("accept", "application/json"),
            ("x-correlation-id", "corr-42"),
        ],
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        upstream.values("host"),
        vec![upstream.authority().to_string()],
        "Host must come from the backend URL, not the client"
    );
    for hop in [
        "keep-alive",
        "proxy-authorization",
        "te",
        "trailer",
        "upgrade",
        "x-hop-custom",
    ] {
        assert!(
            !upstream.has(hop),
            "{} reached the upstream: {:?}",
            hop,
            upstream.values(hop)
        );
    }
    assert_eq!(
        upstream.values("accept"),
        vec!["application/json".to_string()]
    );
    assert_eq!(
        upstream.values("x-correlation-id"),
        vec!["corr-42".to_string()]
    );
}

/// Mesh egress is injected by the executor when the backend URL matches a
/// remote ingress of a mesh that lists this pipeline's egress.
fn mesh_egress_extra(base_url: &str) -> String {
    format!(
        r#"
        [mesh.partner]
        type = "http"
        provider = "local"
        jwt_secret = "{secret}"
        ingress = ["partner_ingress"]
        egress = ["core_egress"]

        [remote_ingress.partner_ingress]
        urls = ["{base_url}"]

        [egress.core_egress]
        pipeline = "core"
        type = "http"
    "#,
        secret = MESH_SECRET,
    )
}

async fn assert_mesh_egress_sends_only_mesh_jwt(backend_options: &str) {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        "http",
        &upstream.base_url,
        "",
        backend_options,
        &mesh_egress_extra(&upstream.base_url),
    ));

    let status = send(config, &[("authorization", CLIENT_AUTH)]).await;
    assert_eq!(status, StatusCode::OK);

    let auth = upstream.values("authorization");
    assert_eq!(auth.len(), 1, "expected one Authorization, got {:?}", auth);
    let token = auth[0]
        .strip_prefix("Bearer ")
        .unwrap_or_else(|| panic!("mesh JWT should be a Bearer token, got {}", auth[0]));

    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.validate_aud = false;
    let claims = jsonwebtoken::decode::<serde_json::Value>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(MESH_SECRET.as_bytes()),
        &validation,
    )
    .expect("upstream Authorization should be the mesh JWT signed with the mesh secret")
    .claims;
    assert_eq!(claims["mesh_id"], "partner");
}

#[tokio::test]
#[serial_test::serial]
async fn mesh_egress_carries_mesh_jwt_and_not_client_auth() {
    assert_mesh_egress_sends_only_mesh_jwt("").await;
}

#[tokio::test]
#[serial_test::serial]
async fn mesh_egress_jwt_wins_over_forwarded_client_auth() {
    assert_mesh_egress_sends_only_mesh_jwt("forward_authorization = true").await;
}

async fn stream_through_backend(
    upstream: &Upstream,
    options: HashMap<String, serde_json::Value>,
) -> usize {
    let headers = HashMap::from([
        ("host".to_string(), "client-facing.example.com".to_string()),
        ("authorization".to_string(), CLIENT_AUTH.to_string()),
        ("connection".to_string(), "x-hop-custom".to_string()),
        (
            "x-hop-custom".to_string(),
            "named-in-connection".to_string(),
        ),
        ("x-correlation-id".to_string(), "corr-42".to_string()),
    ]);
    let envelope = RequestEnvelope::<Vec<u8>>::builder()
        .method("GET")
        .uri("/proxy/resource")
        .headers(headers)
        .metadata(HashMap::from([(
            "path".to_string(),
            "resource".to_string(),
        )]))
        .original_data(Vec::new())
        .build()
        .unwrap();

    let backend = HttpStreamingBackend {
        base_url: None,
        format: None,
        chunk_timeout_secs: None,
        max_chunk_size: None,
    };
    let mut options = options;
    options.insert(
        "base_url".to_string(),
        serde_json::Value::String(upstream.base_url.clone()),
    );
    let stream = backend
        .backend_outgoing_stream(envelope, &options)
        .await
        .expect("streaming request");
    stream.count().await
}

#[tokio::test]
#[serial_test::serial]
async fn http_streaming_backend_obeys_the_same_rules() {
    let upstream = start_recording_upstream().await;
    let items = stream_through_backend(&upstream, HashMap::new()).await;

    assert_eq!(items, 1);
    assert!(!upstream.has("authorization"));
    assert!(!upstream.has("x-hop-custom"));
    assert_eq!(
        upstream.values("host"),
        vec![upstream.authority().to_string()]
    );
    assert_eq!(
        upstream.values("x-correlation-id"),
        vec!["corr-42".to_string()]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn http_streaming_backend_forwards_client_auth_when_enabled() {
    let upstream = start_recording_upstream().await;
    let options = HashMap::from([(
        "forward_authorization".to_string(),
        serde_json::Value::Bool(true),
    )]);
    stream_through_backend(&upstream, options).await;

    assert_eq!(
        upstream.values("authorization"),
        vec![CLIENT_AUTH.to_string()]
    );
}
