use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::Router;
use harmony::config::config::{Config, ConfigError};
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceExt;

/// Load config from TOML string
fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let mut config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    
    // Resolve references (including authentication)
    harmony::config::resolution::resolve_references(&mut config)
        .expect("Failed to resolve references");
    
    Ok(config)
}

/// Mock upstream server that echoes received headers
async fn build_mock_echo_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/echo",
        post(|headers: axum::http::HeaderMap, body: axum::body::Bytes| async move {
            let body_json: serde_json::Value =
                serde_json::from_slice(&body).unwrap_or(json!({}));
            let mut header_map = std::collections::HashMap::new();
            for (key, value) in headers.iter() {
                if let Ok(v) = value.to_str() {
                    header_map.insert(key.to_string(), v.to_string());
                }
            }
            axum::Json(json!({
                "body": body_json,
                "headers": header_map
            }))
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    (base_url, handle)
}

#[tokio::test]
async fn test_backend_bearer_auth_header() {
    let (backend_url, _handle) = build_mock_echo_server().await;
    let _ = std::fs::create_dir_all("../../tmp");

    let config_str = format!(
        r#"
        [proxy]
        id = "backend-auth-header-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8082

        # Global authentication
        [authentications.bearer-auth]
        id = "bearer-auth"
        method = "bearer"
        [authentications.bearer-auth.options]
        token = "test-bearer-token-12345"

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["http_backend"]
        middleware = ["passthru"]

        [middleware.passthru]
        type = "passthru"

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.http_backend]
        service = "http"
        authentication = "bearer-auth"
        [backends.http_backend.options]
        base_url = "{}"

        [services.http]
        module = ""

        [middleware_types.passthru]
        module = ""
    "#,
        backend_url
    );

    let cfg = load_config_from_str(&config_str).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let payload = json!({"test": "data"});

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    // Verify the Authorization header was added by the backend
    let headers = json["headers"].as_object().expect("headers object");
    assert_eq!(
        headers.get("authorization").and_then(|v| v.as_str()),
        Some("Bearer test-bearer-token-12345"),
        "Backend should add Bearer authorization header"
    );
}

#[tokio::test]
async fn test_backend_basic_auth_header() {
    let (backend_url, _handle) = build_mock_echo_server().await;
    let _ = std::fs::create_dir_all("../../tmp");

    let config_str = format!(
        r#"
        [proxy]
        id = "backend-auth-header-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8083

        # Global authentication
        [authentications.basic-auth]
        id = "basic-auth"
        method = "basic"
        [authentications.basic-auth.options]
        username = "testuser"
        password = "testpass"

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["http_backend"]
        middleware = ["passthru"]

        [middleware.passthru]
        type = "passthru"

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.http_backend]
        service = "http"
        authentication = "basic-auth"
        [backends.http_backend.options]
        base_url = "{}"

        [services.http]
        module = ""

        [middleware_types.passthru]
        module = ""
    "#,
        backend_url
    );

    let cfg = load_config_from_str(&config_str).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let payload = json!({"test": "data"});

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    // Verify the Authorization header was added by the backend
    let headers = json["headers"].as_object().expect("headers object");
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.as_str())
        .expect("Authorization header should be present");

    // Basic auth should be base64-encoded "testuser:testpass"
    let expected = format!("Basic {}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"testuser:testpass"));
    assert_eq!(auth_header, expected, "Backend should add Basic authorization header");
}

#[tokio::test]
async fn test_backend_api_key_header() {
    let (backend_url, _handle) = build_mock_echo_server().await;
    let _ = std::fs::create_dir_all("../../tmp");

    let config_str = format!(
        r#"
        [proxy]
        id = "backend-auth-header-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8084

        # Global authentication
        [authentications.api-key-auth]
        id = "api-key-auth"
        method = "api_key"
        [authentications.api-key-auth.options]
        api_key = "my-secret-api-key-123"
        header_name = "X-API-Key"

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["http_backend"]
        middleware = ["passthru"]

        [middleware.passthru]
        type = "passthru"

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.http_backend]
        service = "http"
        authentication = "api-key-auth"
        [backends.http_backend.options]
        base_url = "{}"

        [services.http]
        module = ""

        [middleware_types.passthru]
        module = ""
    "#,
        backend_url
    );

    let cfg = load_config_from_str(&config_str).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let payload = json!({"test": "data"});

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    // Verify the API key header was added by the backend
    let headers = json["headers"].as_object().expect("headers object");
    assert_eq!(
        headers.get("x-api-key").and_then(|v| v.as_str()),
        Some("my-secret-api-key-123"),
        "Backend should add X-API-Key header"
    );
}

#[tokio::test]
async fn test_backend_custom_api_key_header() {
    let (backend_url, _handle) = build_mock_echo_server().await;
    let _ = std::fs::create_dir_all("../../tmp");

    let config_str = format!(
        r#"
        [proxy]
        id = "backend-auth-header-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8085

        # Global authentication with custom header name
        [authentications.custom-key-auth]
        id = "custom-key-auth"
        method = "api_key"
        [authentications.custom-key-auth.options]
        api_key = "custom-key-value"
        header_name = "X-Custom-Auth-Key"

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["http_backend"]
        middleware = ["passthru"]

        [middleware.passthru]
        type = "passthru"

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.http_backend]
        service = "http"
        authentication = "custom-key-auth"
        [backends.http_backend.options]
        base_url = "{}"

        [services.http]
        module = ""

        [middleware_types.passthru]
        module = ""
    "#,
        backend_url
    );

    let cfg = load_config_from_str(&config_str).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let payload = json!({"test": "data"});

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    // Verify the custom API key header was added by the backend
    let headers = json["headers"].as_object().expect("headers object");
    assert_eq!(
        headers.get("x-custom-auth-key").and_then(|v| v.as_str()),
        Some("custom-key-value"),
        "Backend should add custom authentication header"
    );
}

#[tokio::test]
async fn test_backend_no_auth_no_header() {
    let (backend_url, _handle) = build_mock_echo_server().await;
    let _ = std::fs::create_dir_all("../../tmp");

    let config_str = format!(
        r#"
        [proxy]
        id = "backend-auth-header-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8086

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_endpoint"]
        backends = ["http_backend"]
        middleware = ["passthru"]

        [middleware.passthru]
        type = "passthru"

        [endpoints.http_endpoint]
        service = "http"
        [endpoints.http_endpoint.options]
        path_prefix = "/proxy"

        [backends.http_backend]
        service = "http"
        [backends.http_backend.options]
        base_url = "{}"

        [services.http]
        module = ""

        [middleware_types.passthru]
        module = ""
    "#,
        backend_url
    );

    let cfg = load_config_from_str(&config_str).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    let payload = json!({"test": "data"});

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/echo")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    // Verify no authentication headers were added
    let headers = json["headers"].as_object().expect("headers object");
    assert!(
        headers.get("authorization").is_none(),
        "Backend without auth should not add Authorization header"
    );
    assert!(
        headers.get("x-api-key").is_none(),
        "Backend without auth should not add X-API-Key header"
    );
}
