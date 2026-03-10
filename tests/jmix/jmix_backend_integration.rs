// Integration tests for JMIX backend service
//
// Test Status:
// ✅ test_jmix_backend_post_forwarding - PASSING
// ✅ test_jmix_backend_get_manifest_forwarding - PASSING  
// ✅ test_jmix_backend_get_forwarding - PASSING (fixed in envelope.rs)
//
// Previously, the GET forwarding test failed because application/zip was not recognized
// as a binary content type in envelope.rs:to_json(). This caused ZIP file content to be
// lost during middleware processing. Fixed by adding archive content types to the
// is_binary check in src/models/envelope/envelope.rs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use harmony::config::config::{Config, ConfigError};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

async fn build_test_router(config_str: &str) -> Router<()> {
    let _ = std::fs::create_dir_all("./tmp");
    let cfg = load_config_from_str(config_str).expect("valid config");
    harmony::router::build_network_router(Arc::new(cfg), "default").await
}

/// Mock upstream JMIX server
struct MockJmixServer {
    envelopes: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
}

impl MockJmixServer {
    fn new() -> Self {
        Self {
            envelopes: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    fn router(self) -> Router {
        let envelopes = self.envelopes.clone();

        Router::new()
            .route(
                "/api/jmix",
                post({
                    let envelopes = envelopes.clone();
                    move |body: axum::body::Bytes| {
                        let envelopes = envelopes.clone();
                        async move {
                            // Store the uploaded envelope
                            let id = uuid::Uuid::new_v4().to_string();
                            let mut store = envelopes.lock().await;
                            store.insert(id.clone(), body.to_vec());
                            drop(store);

                            axum::Json(json!({
                                "id": id,
                                "status": "stored"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/api/jmix/{id}",
                get({
                    let envelopes = envelopes.clone();
                    move |axum::extract::Path(id): axum::extract::Path<String>| {
                        let envelopes = envelopes.clone();
                        async move {
                            eprintln!("Mock server: GET /api/jmix/{}", id);
                            let store = envelopes.lock().await;
                            if let Some(data) = store.get(&id) {
                                eprintln!("Mock server: Found envelope, returning {} bytes", data.len());
                                (
                                    StatusCode::OK,
                                    [("content-type", "application/zip")],
                                    data.clone(),
                                )
                            } else {
                                eprintln!("Mock server: Envelope not found");
                                (
                                    StatusCode::NOT_FOUND,
                                    [("content-type", "application/json")],
                                    b"not found".to_vec(),
                                )
                            }
                        }
                    }
                }),
            )
            .route(
                "/api/jmix/{id}/manifest",
                get({
                    let envelopes = envelopes.clone();
                    move |axum::extract::Path(id): axum::extract::Path<String>| {
                        let envelopes = envelopes.clone();
                        async move {
                            let store = envelopes.lock().await;
                            if store.contains_key(&id) {
                                axum::Json(json!({
                                    "id": id,
                                    "type": "envelope",
                                    "version": 1
                                }))
                            } else {
                                axum::Json(json!({
                                    "error": "not found"
                                }))
                            }
                        }
                    }
                }),
            )
    }
}

/// Spawn a test server and return its address
async fn spawn_test_server(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().expect("Failed to get local addr");

    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("Test server failed");
    });

    // Give the server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    addr
}

fn get_jmix_backend_config(backend_base_url: &str) -> String {
    format!(
        r#"
[proxy]
id = "jmix-backend-test"
log_level = "info"
store_dir = "./tmp"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8081

[pipelines.core]
description = "JMIX backend test pipeline"
networks = ["default"]
endpoints = ["http_endpoint"]
backends = ["jmix_backend"]

[endpoints.http_endpoint]
service = "http"
[endpoints.http_endpoint.options]
path_prefix = "/jmix"

[backends.jmix_backend]
service = "jmix_backend"
[backends.jmix_backend.options]
base_url = "{}"

[services.http]
module = ""

[services.jmix_backend]
module = ""
"#,
        backend_base_url
    )
}

#[tokio::test]
async fn test_jmix_backend_post_forwarding() {
    // Reset global storage
    harmony::globals::reset_storage();

    // Start mock upstream JMIX server
    let mock_server = MockJmixServer::new();
    let upstream_addr = spawn_test_server(mock_server.router()).await;
    let backend_url = format!("http://{}", upstream_addr);

    let config_str = get_jmix_backend_config(&backend_url);
    let app = build_test_router(&config_str).await;

    // Create a fake JMIX envelope ZIP body
    let zip_body = b"fake-zip-content";

    // Send POST request through proxy (will be forwarded to /api/jmix on upstream)
    let request = Request::builder()
        .method("POST")
        .uri("/jmix/api/jmix")
        .header("content-type", "application/zip")
        .body(Body::from(&zip_body[..]))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    
    let status = response.status();
    
    // Debug: print status and body if not 200
    if status != StatusCode::OK {
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        eprintln!("Status: {}", status);
        eprintln!("Body: {}", String::from_utf8_lossy(&body_bytes));
        panic!("Expected 200 OK, got {}", status);
    }

    // Should get 200 OK with JSON response from upstream
    assert_eq!(status, StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(body_json["status"], "stored");
    assert!(body_json["id"].is_string());
}

#[tokio::test]
async fn test_jmix_backend_get_forwarding() {
    // Reset global storage
    harmony::globals::reset_storage();

    // Start mock upstream JMIX server
    let mock_server = MockJmixServer::new();

    // Pre-populate the mock server with an envelope
    let test_id = uuid::Uuid::new_v4().to_string();
    let test_data = b"test-envelope-zip-data";
    {
        let mut store = mock_server.envelopes.lock().await;
        store.insert(test_id.clone(), test_data.to_vec());
    }

    let upstream_addr = spawn_test_server(mock_server.router()).await;
    let backend_url = format!("http://{}", upstream_addr);

    let config_str = get_jmix_backend_config(&backend_url);
    let app = build_test_router(&config_str).await;

    // Send GET request through proxy (will be forwarded to /api/jmix/{id} on upstream)
    let request = Request::builder()
        .method("GET")
        .uri(format!("/jmix/api/jmix/{}", test_id))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should get 200 OK with ZIP data from upstream
    let status = response.status();
    eprintln!("Response status: {}", status);
    eprintln!("Response headers: {:?}", response.headers());
    assert_eq!(status, StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    eprintln!("Expected {} bytes, got {} bytes", test_data.len(), body_bytes.len());
    if body_bytes.is_empty() {
        eprintln!("ERROR: Response body is empty!");
    }

    assert_eq!(body_bytes.as_ref(), test_data);
}

#[tokio::test]
async fn test_jmix_backend_get_manifest_forwarding() {
    // Reset global storage
    harmony::globals::reset_storage();

    // Start mock upstream JMIX server
    let mock_server = MockJmixServer::new();

    // Pre-populate the mock server with an envelope
    let test_id = uuid::Uuid::new_v4().to_string();
    {
        let mut store = mock_server.envelopes.lock().await;
        store.insert(test_id.clone(), b"dummy".to_vec());
    }

    let upstream_addr = spawn_test_server(mock_server.router()).await;
    let backend_url = format!("http://{}", upstream_addr);

    let config_str = get_jmix_backend_config(&backend_url);
    let app = build_test_router(&config_str).await;

    // Send GET manifest request through proxy (will be forwarded to /api/jmix/{id}/manifest on upstream)
    let request = Request::builder()
        .method("GET")
        .uri(format!("/jmix/api/jmix/{}/manifest", test_id))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should get 200 OK with manifest JSON from upstream
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(manifest["id"], test_id);
    assert_eq!(manifest["type"], "envelope");
    assert_eq!(manifest["version"], 1);
}
