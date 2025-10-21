use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{delete, get, post, put};
use axum::Router;
use harmony::config::config::{Config, ConfigError};
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceExt;

/// Load config from TOML string
fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

/// Build a mock upstream HTTP server for testing
async fn build_mock_upstream_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/api/users", get(|| async {
            axum::Json(json!({
                "users": [
                    {"id": 1, "name": "Alice"},
                    {"id": 2, "name": "Bob"}
                ]
            }))
        }))
        .route("/api/users", post(|body: axum::body::Bytes| async move {
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!({}));
            axum::Json(json!({
                "success": true,
                "created": json
            }))
        }))
        .route("/api/users/{id}", put(|axum::extract::Path(id): axum::extract::Path<u32>, body: axum::body::Bytes| async move {
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!({}));
            axum::Json(json!({
                "success": true,
                "id": id,
                "updated": json
            }))
        }))
        .route("/api/users/{id}", delete(|axum::extract::Path(id): axum::extract::Path<u32>| async move {
            axum::Json(json!({
                "success": true,
                "deleted_id": id
            }))
        }))
        .route("/api/echo", post(|headers: axum::http::HeaderMap, body: axum::body::Bytes| async move {
            let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!({}));
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
        }))
        .route("/api/query", get(|axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            axum::Json(json!({
                "query_params": params
            }))
        }))
        .route("/api/status/{code}", get(|axum::extract::Path(code): axum::extract::Path<u16>| async move {
            let status = axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::OK);
            (status, axum::Json(json!({
                "status": code
            })))
        }));

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

/// Get test config for HTTP backend
fn get_http_backend_config(backend_base_url: &str) -> String {
    format!(
        r#"
        [proxy]
        id = "http-backend-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8081

        [pipelines.core]
        description = "HTTP->HTTP backend pipeline"
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
        backend_base_url
    )
}

async fn build_test_router(backend_base_url: &str) -> Router<()> {
    let _ = std::fs::create_dir_all("../../tmp");
    let config_str = get_http_backend_config(backend_base_url);
    let cfg = load_config_from_str(&config_str).expect("valid config");
    harmony::router::build_network_router(Arc::new(cfg), "default").await
}

#[tokio::test]
async fn test_http_backend_get_request() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/users")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    assert!(json["users"].is_array());
    assert_eq!(json["users"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_http_backend_post_request() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    let payload = json!({
        "name": "Charlie",
        "email": "charlie@example.com"
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/users")
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

    assert_eq!(json["success"], true);
    assert_eq!(json["created"]["name"], "Charlie");
}

#[tokio::test]
async fn test_http_backend_put_request() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    let payload = json!({
        "name": "Charlie Updated",
        "email": "charlie.new@example.com"
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/users/123")
                .method("PUT")
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

    assert_eq!(json["success"], true);
    assert_eq!(json["id"], 123);
    assert_eq!(json["updated"]["name"], "Charlie Updated");
}

#[tokio::test]
async fn test_http_backend_delete_request() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/users/456")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(json["success"], true);
    assert_eq!(json["deleted_id"], 456);
}

#[tokio::test]
async fn test_http_backend_forwards_headers() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    let payload = json!({"test": "data"});

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/echo")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-custom-header", "test-value")
                .header("authorization", "Bearer token123")
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

    assert_eq!(json["body"]["test"], "data");
    
    // Check headers were forwarded
    let headers = json["headers"].as_object().expect("headers object");
    assert_eq!(headers.get("x-custom-header").and_then(|v| v.as_str()), Some("test-value"));
    assert_eq!(headers.get("authorization").and_then(|v| v.as_str()), Some("Bearer token123"));
}

#[tokio::test]
async fn test_http_backend_query_params() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/query?foo=bar&baz=qux&name=test")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    let query_params = json["query_params"].as_object().expect("query_params object");
    assert_eq!(query_params.get("foo").and_then(|v| v.as_str()), Some("bar"));
    assert_eq!(query_params.get("baz").and_then(|v| v.as_str()), Some("qux"));
    assert_eq!(query_params.get("name").and_then(|v| v.as_str()), Some("test"));
}

#[tokio::test]
async fn test_http_backend_status_codes() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    // Test 404
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/proxy/api/status/404")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Test 500
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/proxy/api/status/500")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // Test 201
    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/status/201")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_http_backend_empty_body() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/users")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
    
    // Should still get valid JSON response
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert!(json["users"].is_array());
}

#[tokio::test]
async fn test_http_backend_json_content_type() {
    let (backend_url, _handle) = build_mock_upstream_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/api/users")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Check that content-type header is preserved
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    
    assert!(content_type.is_some());
    assert!(content_type.unwrap().contains("application/json"));
}
