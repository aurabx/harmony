//! Which client headers reach the upstream through a backend.
//!
//! A recording upstream captures every header it receives, including repeated
//! ones, so the tests can prove that the upstream gets exactly one
//! `Authorization` header and never the client's credential by default.
//! Linear: AURA-2433, AURA-2411, AURA-2427, AURA-2584.

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
const API_KEY_HEADER: &str = "X-Harmony-Key";
const CLIENT_API_KEY_HEADER: &str = "X-HARMONY-KEY";
const TARGET_API_KEY: &str = "target-api-key-456";
const CLIENT_API_KEY: &str = "client-api-key-789";

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

        [authentications.target-key]
        id = "target-key"
        method = "api_key"
        [authentications.target-key.options]
        api_key = "{api_key}"
        header_name = "{api_key_header}"

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
        api_key = TARGET_API_KEY,
        api_key_header = API_KEY_HEADER,
    )
}

async fn send(config: Config, headers: &[(&str, &str)]) -> StatusCode {
    send_to(config, "/proxy/resource", headers).await
}

async fn send_to(config: Config, uri: &str, headers: &[(&str, &str)]) -> StatusCode {
    let config = Arc::new(config);
    harmony::globals::set_config(config.clone());
    let app = harmony::router::build_network_router(config, "default").await;
    let mut request = Request::builder().uri(uri).method("GET");
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

async fn assert_client_api_key_is_replaced(service: &str, extra: &str) {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        service,
        &upstream.base_url,
        r#"authentication = "target-key""#,
        "",
        extra,
    ));

    let status = send(
        config,
        &[
            (CLIENT_API_KEY_HEADER, CLIENT_API_KEY),
            ("x_harmony_key", CLIENT_API_KEY),
            ("x-correlation-id", "corr-42"),
        ],
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        upstream.values(&API_KEY_HEADER.to_ascii_lowercase()),
        vec![TARGET_API_KEY.to_string()]
    );
    assert!(!upstream.has("x_harmony_key"));
    assert_eq!(
        upstream.values("x-correlation-id"),
        vec!["corr-42".to_string()]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn http_client_api_key_never_reaches_the_upstream() {
    assert_client_api_key_is_replaced("http", "").await;
}

#[tokio::test]
#[serial_test::serial]
async fn fhir_client_api_key_never_reaches_the_upstream() {
    assert_client_api_key_is_replaced("fhir", "").await;
}

#[tokio::test]
#[serial_test::serial]
async fn jmix_client_api_key_never_reaches_the_upstream() {
    assert_client_api_key_is_replaced("jmix_backend", JMIX_SERVICE).await;
}

const JMIX_SERVICE: &str = r#"
    [services.jmix_backend]
    module = ""
"#;

async fn assert_target_bearer_replaces_mesh_jwt(service: &str, extra: &str) {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        service,
        &upstream.base_url,
        r#"authentication = "target-bearer""#,
        "",
        &format!("{}\n{}", mesh_egress_extra(&upstream.base_url), extra),
    ));

    let status = send(config, &[("authorization", CLIENT_AUTH)]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        upstream.values("authorization"),
        vec![format!("Bearer {}", TARGET_TOKEN)]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn http_target_bearer_replaces_mesh_jwt() {
    assert_target_bearer_replaces_mesh_jwt("http", "").await;
}

#[tokio::test]
#[serial_test::serial]
async fn fhir_target_bearer_replaces_mesh_jwt() {
    assert_target_bearer_replaces_mesh_jwt("fhir", "").await;
}

#[tokio::test]
#[serial_test::serial]
async fn jmix_target_bearer_replaces_mesh_jwt() {
    assert_target_bearer_replaces_mesh_jwt("jmix_backend", JMIX_SERVICE).await;
}

#[tokio::test]
#[serial_test::serial]
async fn target_api_key_is_sent_beside_mesh_jwt() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        "http",
        &upstream.base_url,
        r#"authentication = "target-key""#,
        "",
        &mesh_egress_extra(&upstream.base_url),
    ));

    let status = send(config, &[]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        upstream.values(&API_KEY_HEADER.to_ascii_lowercase()),
        vec![TARGET_API_KEY.to_string()]
    );
    assert_eq!(upstream.values("authorization").len(), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn client_connection_header_cannot_remove_the_mesh_jwt() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        "http",
        &upstream.base_url,
        "",
        "",
        &mesh_egress_extra(&upstream.base_url),
    ));

    let status = send(config, &[("connection", "authorization")]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream.values("authorization").len(), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn dot_segment_paths_are_rejected_before_the_backend() {
    let upstream = start_recording_upstream().await;

    for uri in [
        "/proxy/x/../y",
        "/proxy/x/%2e%2e/y",
        "/proxy/x/%2E%2E/y",
        "/proxy/x/.%2e/y",
        "/proxy/./y",
    ] {
        let config = load_config(&proxy_config(
            "http",
            &upstream.base_url,
            r#"authentication = "target-key""#,
            "",
            "",
        ));
        assert_eq!(
            send_to(config, uri, &[]).await,
            StatusCode::BAD_REQUEST,
            "{}",
            uri
        );
    }
    assert!(upstream.recorded.lock().unwrap().is_empty());
}

/// An upstream that answers every request with a redirect to `location`.
async fn start_redirecting_upstream(location: String) -> String {
    let app = Router::new().route(
        "/{*path}",
        any(move || {
            let location = location.clone();
            async move { (StatusCode::FOUND, [("location", location)], "") }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    base_url
}

async fn assert_credential_stays_on_cross_origin_redirect(service: &str, extra: &str) {
    let other_origin = start_recording_upstream().await;
    let redirecting = start_redirecting_upstream(format!("{}/stolen", other_origin.base_url)).await;
    let config = load_config(&proxy_config(
        service,
        &redirecting,
        r#"authentication = "target-key""#,
        "",
        extra,
    ));

    let status = send(config, &[]).await;

    assert_eq!(status, StatusCode::FOUND);
    assert!(other_origin.recorded.lock().unwrap().is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn http_does_not_follow_cross_origin_redirect_with_credential() {
    assert_credential_stays_on_cross_origin_redirect("http", "").await;
}

#[tokio::test]
#[serial_test::serial]
async fn fhir_does_not_follow_cross_origin_redirect_with_credential() {
    assert_credential_stays_on_cross_origin_redirect("fhir", "").await;
}

#[tokio::test]
#[serial_test::serial]
async fn jmix_does_not_follow_cross_origin_redirect_with_credential() {
    assert_credential_stays_on_cross_origin_redirect("jmix_backend", JMIX_SERVICE).await;
}

#[tokio::test]
#[serial_test::serial]
async fn same_origin_redirect_is_followed_with_credential() {
    let app = Router::new()
        .route(
            "/resource",
            any(|| async { (StatusCode::FOUND, [("location", "/moved")], "") }),
        )
        .route(
            "/moved",
            any(|headers: axum::http::HeaderMap| async move {
                let key = headers
                    .get(API_KEY_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                if key == TARGET_API_KEY {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let config = load_config(&proxy_config(
        "http",
        &base_url,
        r#"authentication = "target-key""#,
        "",
        "",
    ));

    assert_eq!(send(config, &[]).await, StatusCode::OK);
}

#[tokio::test]
#[serial_test::serial]
async fn http_streaming_backend_sends_authentication_from_config() {
    let upstream = start_recording_upstream().await;
    let config = load_config(&proxy_config(
        "http_streaming_backend",
        &upstream.base_url,
        r#"authentication = "target-key""#,
        "",
        r#"
        [services.http_streaming_backend]
        module = ""
        "#,
    ));

    let status = send(config, &[(CLIENT_API_KEY_HEADER, CLIENT_API_KEY)]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        upstream.values(&API_KEY_HEADER.to_ascii_lowercase()),
        vec![TARGET_API_KEY.to_string()]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn http_streaming_does_not_follow_cross_origin_redirect_with_credential() {
    let other_origin = start_recording_upstream().await;
    let redirecting = start_redirecting_upstream(format!("{}/stolen", other_origin.base_url)).await;
    let mut options = streaming_auth_option(target_key_definition());
    options.insert(
        "base_url".to_string(),
        serde_json::Value::String(redirecting),
    );

    let result = open_stream(options, HashMap::new()).await;

    assert!(result.is_err());
    assert!(other_origin.recorded.lock().unwrap().is_empty());
}

fn streaming_auth_option(auth_definition: serde_json::Value) -> HashMap<String, serde_json::Value> {
    HashMap::from([("authentication_def".to_string(), auth_definition)])
}

fn target_key_definition() -> serde_json::Value {
    serde_json::json!({
        "id": "target-key",
        "method": "api_key",
        "options": { "api_key": TARGET_API_KEY, "header_name": API_KEY_HEADER },
    })
}

#[tokio::test]
#[serial_test::serial]
async fn http_streaming_client_api_key_never_reaches_the_upstream() {
    let upstream = start_recording_upstream().await;
    let headers = HashMap::from([(
        CLIENT_API_KEY_HEADER.to_string(),
        CLIENT_API_KEY.to_string(),
    )]);

    stream_headers_through_backend(
        &upstream,
        streaming_auth_option(target_key_definition()),
        headers,
    )
    .await;

    assert_eq!(
        upstream.values(&API_KEY_HEADER.to_ascii_lowercase()),
        vec![TARGET_API_KEY.to_string()]
    );
}

#[tokio::test]
#[serial_test::serial]
async fn http_streaming_backend_sends_target_auth_exactly_once() {
    let upstream = start_recording_upstream().await;
    let mut options = streaming_auth_option(serde_json::json!({
        "id": "target-bearer",
        "method": "bearer",
        "options": { "token": TARGET_TOKEN },
    }));
    options.insert(
        "forward_authorization".to_string(),
        serde_json::Value::Bool(true),
    );

    stream_through_backend(&upstream, options).await;

    assert_eq!(
        upstream.values("authorization"),
        vec![format!("Bearer {}", TARGET_TOKEN)]
    );
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
    stream_headers_through_backend(upstream, options, headers).await
}

async fn stream_headers_through_backend(
    upstream: &Upstream,
    mut options: HashMap<String, serde_json::Value>,
    headers: HashMap<String, String>,
) -> usize {
    options.insert(
        "base_url".to_string(),
        serde_json::Value::String(upstream.base_url.clone()),
    );
    open_stream(options, headers)
        .await
        .expect("streaming request")
        .count()
        .await
}

async fn open_stream(
    options: HashMap<String, serde_json::Value>,
    headers: HashMap<String, String>,
) -> Result<harmony::models::services::streaming::ResponseStream, String> {
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
    backend
        .backend_outgoing_stream(envelope, &options)
        .await
        .map_err(|e| e.to_string())
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
