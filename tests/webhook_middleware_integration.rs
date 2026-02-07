use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::post;
use axum::Router;
use harmony::config::config::{Config, ConfigError};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use base64::Engine as _;

#[derive(Clone, Default)]
struct WebhookStore {
    inner: Arc<tokio::sync::Mutex<Vec<(Value, HashMap<String, String>)>>>,
}

#[tokio::test]
async fn test_webhook_non_blocking_on_error() {
    // Use an unreachable endpoint to force webhook send failure
    let cfg_text = r#"
        [proxy]
        id = "webhook-error-tolerance"
        log_level = "info"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8089

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["echo_backend"]
        middleware = ["hook_fail"]

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.echo_backend]
        service = "echo"

        [middleware.hook_fail]
        type = "webhook"
        [middleware.hook_fail.options]
        endpoint = "http://127.0.0.1:9/hook"  # almost certainly closed
        apply = "left"

        [services.http]
        module = ""
        [services.echo]
        module = ""
        [middleware_types.webhook]
        module = ""
        [middleware_types.passthru]
        module = ""
    "#;

    let cfg = load_config_from_str(cfg_text).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // The request should succeed quickly even though webhook send fails in background
    let start = std::time::Instant::now();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");
    let elapsed_ms = start.elapsed().as_millis();

    assert_eq!(response.status(), StatusCode::OK);
    // Should return well under the 5s webhook timeout since it's fire-and-forget
    assert!(elapsed_ms < 1000, "request took too long: {}ms", elapsed_ms);
}

async fn webhook_handler(
    State(store): State<WebhookStore>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    let mut header_map = HashMap::new();
    for (k, v) in headers.iter() {
        if let Ok(s) = v.to_str() {
            header_map.insert(k.to_string(), s.to_string());
        }
    }
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(json!({}));

    let mut lock = store.inner.lock().await;
    lock.push((body_json, header_map));
    StatusCode::OK
}

async fn build_webhook_receiver() -> (String, WebhookStore, JoinHandle<()>) {
    let store = WebhookStore::default();
    let app = Router::new()
        .route("/hook", post(webhook_handler))
        .with_state(store.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    (base_url, store, handle)
}

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let mut cfg: Config = toml::from_str(toml).expect("parse TOML");
    cfg.validate()?;
    // Resolve references (ensures targets/peers etc. but safe even if unused)
    harmony::config::resolution::resolve_references(&mut cfg).expect("resolve refs");
    Ok(cfg)
}

async fn wait_for_posts(store: &WebhookStore, at_least: usize, timeout_ms: u64) -> Vec<(Value, HashMap<String, String>)> {
    let mut waited = 0u64;
    loop {
        {
            let lock = store.inner.lock().await;
            if lock.len() >= at_least {
                return lock.clone();
            }
        }
        if waited >= timeout_ms { break; }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        waited += 20;
    }
    store.inner.lock().await.clone()
}

#[tokio::test]
async fn test_webhook_left_posts_payload_with_redaction_and_auth() {
    let (hook_url, store, _handle) = build_webhook_receiver().await;

    // Temporary JOLT spec that writes metadata for this webhook instance
    let spec_text = r#"[
      { "operation": "default", "spec": {
          "webhook.hook_left": "{\\"from\\":\\"metadata_transform\\",\\"ok\\":true}",
          "secret": "supersecret"
      }}
    ]"#;
    let temp_spec = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(temp_spec.path(), spec_text).unwrap();

    let cfg_text = format!(r#"
        [proxy]
        id = "webhook-left-test"
        log_level = "debug"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8087

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["echo_backend"]
        middleware = ["hook_left"]

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.echo_backend]
        service = "echo"

        # per-instance auth for the webhook
        [authentications.webhook-basic]
        id = "webhook-basic"
        method = "basic"
        [authentications.webhook-basic.options]
        username = "whuser"
        password = "whpass"

        [middleware.hook_left]
        type = "webhook"
        authentication = "webhook-basic"
        [middleware.hook_left.options]
        endpoint = "{hook}/hook"
        apply = "left"
        timeout_secs = 5
        redact_headers = ["authorization", "x-secret"]

        [services.http]
        module = ""
        [services.echo]
        module = ""

        [middleware_types.webhook]
        module = ""
        [middleware_types.passthru]
        module = ""
    "#, hook = hook_url);

    let cfg = load_config_from_str(&cfg_text).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let payload = json!({"hello": "world"});

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("POST")
                .header("content-type", "application/json")
                .header("authorization", "Bearer inbound-token")
                .header("x-secret", "very-secret")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    if response.status() != StatusCode::OK {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        panic!("unexpected status: {} body: {}", StatusCode::from_u16(500).unwrap(), String::from_utf8_lossy(&body));
    }

    // Webhook is fire-and-forget; wait briefly for delivery
    let posts = wait_for_posts(&store, 1, 1500).await;
    assert!(posts.len() >= 1, "expected at least one webhook POST");

    let (body, headers) = &posts[0];

    // Validate payload basics
    assert_eq!(body.get("middleware").and_then(|v| v.as_str()), Some("webhook"));
    assert_eq!(body.get("name").and_then(|v| v.as_str()), Some("hook_left"));
    assert_eq!(body.get("side").and_then(|v| v.as_str()), Some("left"));

    // Redaction checks
    let req_headers = body.get("request").and_then(|r| r.get("headers")).and_then(|h| h.as_object()).expect("headers object");
    assert_eq!(req_headers.get("authorization").and_then(|v| v.as_str()), Some("<redacted>"));
    assert_eq!(req_headers.get("x-secret").and_then(|v| v.as_str()), Some("<redacted>"));

    // Extra should be null when no metadata is provided for this instance
    assert!(body.get("extra").is_some());

    // Webhook call itself should include Basic auth header
    let expected_basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(b"whuser:whpass"),
    );
    assert_eq!(headers.get("authorization").map(|s| s.as_str()), Some(expected_basic.as_str()));
}

#[tokio::test]
async fn test_webhook_both_posts_twice() {
    let (hook_url, store, _handle) = build_webhook_receiver().await;

    let cfg_text = format!(r#"
        [proxy]
        id = "webhook-both-test"
        log_level = "info"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8088

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["echo_backend"]
        middleware = ["hook_both"]

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.echo_backend]
        service = "echo"

        [middleware.hook_both]
        type = "webhook"
        [middleware.hook_both.options]
        endpoint = "{hook}/hook"
        apply = "both"

        [services.http]
        module = ""
        [services.echo]
        module = ""
        [middleware_types.webhook]
        module = ""
        [middleware_types.passthru]
        module = ""
    "#, hook = hook_url);

    let cfg = load_config_from_str(&cfg_text).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Expect two posts: left and right
    let posts = wait_for_posts(&store, 2, 1500).await;
    assert!(posts.len() >= 2, "expected two webhook POSTs");
    let mut sides = posts
        .iter()
        .map(|(b, _)| b.get("side").and_then(|v| v.as_str()).unwrap_or(""))
        .collect::<Vec<_>>();
    sides.sort();
    assert_eq!(sides, vec!["left", "right"]);
}

#[tokio::test]
async fn test_webhook_with_global_sensitive_field_patterns() {
    let (hook_url, store, _handle) = build_webhook_receiver().await;

    // Configure global sensitive_field_patterns at the proxy level
    // These patterns should automatically redact matching fields in webhook payloads
    let cfg_text = format!(r#"
        [proxy]
        id = "webhook-sensitive-patterns-test"
        log_level = "debug"
        sensitive_field_patterns = [
            ".*patient.*name.*",
            ".*ssn.*",
            ".*secret.*"
        ]

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8091

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["echo_backend"]
        middleware = ["hook_patterns"]

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.echo_backend]
        service = "echo"

        [middleware.hook_patterns]
        type = "webhook"
        [middleware.hook_patterns.options]
        endpoint = "{hook}/hook"
        apply = "left"
        # Note: No explicit redact_headers - patterns should handle it

        [services.http]
        module = ""
        [services.echo]
        module = ""
        [middleware_types.webhook]
        module = ""
        [middleware_types.passthru]
        module = ""
    "#, hook = hook_url);

    let cfg = load_config_from_str(&cfg_text).expect("valid config");
    
    // Verify the patterns are loaded in config
    assert_eq!(cfg.proxy.sensitive_field_patterns.len(), 3);
    assert!(cfg.proxy.sensitive_field_patterns.contains(&".*patient.*name.*".to_string()));
    
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Send request with headers that match sensitive patterns
    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("POST")
                .header("content-type", "application/json")
                .header("X-Patient-Name", "John Doe")       // Should be redacted by pattern
                .header("X-Patient-SSN", "123-45-6789")     // Should be redacted by pattern
                .header("X-Api-Secret", "super-secret")     // Should be redacted by pattern
                .header("X-Safe-Header", "visible-value")   // Should NOT be redacted
                .body(Body::from(r#"{"hello":"world"}"#))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Wait for webhook delivery
    let posts = wait_for_posts(&store, 1, 1500).await;
    assert!(posts.len() >= 1, "expected at least one webhook POST");

    let (body, _headers) = &posts[0];

    // Validate that pattern-matched headers are redacted
    let req_headers = body
        .get("request")
        .and_then(|r| r.get("headers"))
        .and_then(|h| h.as_object())
        .expect("headers object");

    // Headers matching sensitive patterns should be redacted
    assert_eq!(
        req_headers.get("x-patient-name").and_then(|v| v.as_str()),
        Some("<redacted>"),
        "X-Patient-Name should be redacted by .*patient.*name.* pattern"
    );
    assert_eq!(
        req_headers.get("x-patient-ssn").and_then(|v| v.as_str()),
        Some("<redacted>"),
        "X-Patient-SSN should be redacted by .*ssn.* pattern"
    );
    assert_eq!(
        req_headers.get("x-api-secret").and_then(|v| v.as_str()),
        Some("<redacted>"),
        "X-Api-Secret should be redacted by .*secret.* pattern"
    );
    
    // Safe header should NOT be redacted
    assert_eq!(
        req_headers.get("x-safe-header").and_then(|v| v.as_str()),
        Some("visible-value"),
        "X-Safe-Header should NOT be redacted"
    );
}

#[tokio::test]
async fn test_webhook_combined_explicit_and_pattern_redaction() {
    let (hook_url, store, _handle) = build_webhook_receiver().await;

    // Test that both explicit redact_headers AND global patterns work together
    let cfg_text = format!(r#"
        [proxy]
        id = "webhook-combined-redaction-test"
        log_level = "debug"
        sensitive_field_patterns = [".*secret.*"]

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8092

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["echo_backend"]
        middleware = ["hook_combined"]

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.echo_backend]
        service = "echo"

        [middleware.hook_combined]
        type = "webhook"
        [middleware.hook_combined.options]
        endpoint = "{hook}/hook"
        apply = "left"
        redact_headers = ["authorization"]  # Explicit redaction

        [services.http]
        module = ""
        [services.echo]
        module = ""
        [middleware_types.webhook]
        module = ""
        [middleware_types.passthru]
        module = ""
    "#, hook = hook_url);

    let cfg = load_config_from_str(&cfg_text).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("POST")
                .header("content-type", "application/json")
                .header("authorization", "Bearer token123")  // Explicit redaction
                .header("x-api-secret", "secret-value")      // Pattern redaction
                .header("x-other", "visible")                // Should NOT be redacted
                .body(Body::from(r#"{"test":"data"}"#))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let posts = wait_for_posts(&store, 1, 1500).await;
    assert!(posts.len() >= 1, "expected at least one webhook POST");

    let (body, _headers) = &posts[0];
    let req_headers = body
        .get("request")
        .and_then(|r| r.get("headers"))
        .and_then(|h| h.as_object())
        .expect("headers object");

    // Both explicit and pattern redaction should work
    assert_eq!(
        req_headers.get("authorization").and_then(|v| v.as_str()),
        Some("<redacted>"),
        "authorization should be redacted by explicit config"
    );
    assert_eq!(
        req_headers.get("x-api-secret").and_then(|v| v.as_str()),
        Some("<redacted>"),
        "x-api-secret should be redacted by pattern"
    );
    assert_eq!(
        req_headers.get("x-other").and_then(|v| v.as_str()),
        Some("visible"),
        "x-other should NOT be redacted"
    );
}
