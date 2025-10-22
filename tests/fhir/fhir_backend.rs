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

/// Build a mock FHIR server for testing
async fn build_mock_fhir_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/Patient", get(|| async {
            axum::Json(json!({
                "resourceType": "Bundle",
                "type": "searchset",
                "total": 2,
                "entry": [
                    {
                        "resource": {
                            "resourceType": "Patient",
                            "id": "1",
                            "name": [{"family": "Smith", "given": ["John"]}]
                        }
                    },
                    {
                        "resource": {
                            "resourceType": "Patient",
                            "id": "2",
                            "name": [{"family": "Doe", "given": ["Jane"]}]
                        }
                    }
                ]
            }))
        }))
        .route("/Patient/{id}", get(|axum::extract::Path(id): axum::extract::Path<String>| async move {
            axum::Json(json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{"family": "Smith", "given": ["John"]}],
                "gender": "male",
                "birthDate": "1970-01-01"
            }))
        }))
        .route("/Patient", post(|body: axum::body::Bytes| async move {
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!({}));
            let id = uuid::Uuid::new_v4().to_string();
            let mut response = json.as_object().cloned().unwrap_or_default();
            response.insert("id".to_string(), json!(id));
            
            (
                StatusCode::CREATED,
                [(axum::http::header::CONTENT_TYPE, "application/fhir+json")],
                axum::Json(json!(response))
            )
        }))
        .route("/Patient/{id}", put(|axum::extract::Path(id): axum::extract::Path<String>, body: axum::body::Bytes| async move {
            let mut json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!({}));
            if let Some(obj) = json.as_object_mut() {
                obj.insert("id".to_string(), json!(id));
            }
            
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/fhir+json")],
                axum::Json(json)
            )
        }))
        .route("/Patient/{id}", delete(|axum::extract::Path(_id): axum::extract::Path<String>| async move {
            (
                StatusCode::NO_CONTENT,
                [(axum::http::header::CONTENT_TYPE, "application/fhir+json")],
                axum::Json(json!({}))
            )
        }))
        .route("/Observation", get(|axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            axum::Json(json!({
                "resourceType": "Bundle",
                "type": "searchset",
                "total": 1,
                "entry": [
                    {
                        "resource": {
                            "resourceType": "Observation",
                            "id": "obs-1",
                            "status": "final",
                            "code": {
                                "coding": [{
                                    "system": "http://loinc.org",
                                    "code": "15074-8"
                                }]
                            },
                            "subject": {
                                "reference": format!("Patient/{}", params.get("patient").unwrap_or(&"unknown".to_string()))
                            }
                        }
                    }
                ],
                "search_params": params
            }))
        }))
        .route("/_status", get(|| async {
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/fhir+json")],
                axum::Json(json!({
                    "status": "operational",
                    "version": "R4"
                }))
            )
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

/// Get test config for FHIR backend
fn get_fhir_backend_config(backend_base_url: &str) -> String {
    format!(
        r#"
        [proxy]
        id = "fhir-backend-test"
        log_level = "debug"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8082

        [pipelines.core]
        description = "FHIR->FHIR backend pipeline"
        networks = ["default"]
        endpoints = ["fhir_endpoint"]
        backends = ["fhir_backend"]
        middleware = ["passthru"]

        [middleware.passthru]
        type = "passthru"

        [endpoints.fhir_endpoint]
        service = "fhir"
        [endpoints.fhir_endpoint.options]
        path_prefix = "/fhir"

        [backends.fhir_backend]
        service = "fhir"
        [backends.fhir_backend.options]
        base_url = "{}"

        [services.fhir]
        module = ""

        [middleware_types.passthru]
        module = ""
    "#,
        backend_base_url
    )
}

async fn build_test_router(backend_base_url: &str) -> Router<()> {
    let _ = std::fs::create_dir_all("../../tmp");
    let config_str = get_fhir_backend_config(backend_base_url);
    let cfg = load_config_from_str(&config_str).expect("valid config");
    harmony::router::build_network_router(Arc::new(cfg), "default").await
}

#[tokio::test]
async fn test_fhir_backend_get_patient() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/Patient/123")
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

    assert_eq!(json["resourceType"], "Patient");
    assert_eq!(json["id"], "123");
    assert_eq!(json["gender"], "male");
}

#[tokio::test]
async fn test_fhir_backend_search_patients() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/Patient")
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

    assert_eq!(json["resourceType"], "Bundle");
    assert_eq!(json["type"], "searchset");
    assert_eq!(json["total"], 2);
    assert!(json["entry"].is_array());
}

#[tokio::test]
async fn test_fhir_backend_create_patient() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let patient = json!({
        "resourceType": "Patient",
        "name": [{
            "family": "Johnson",
            "given": ["Bob"]
        }],
        "gender": "male"
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/Patient")
                .method("POST")
                .header("content-type", "application/fhir+json")
                .body(Body::from(serde_json::to_vec(&patient).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::CREATED);

    // Check FHIR content type is preserved
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert!(content_type.is_some());
    assert!(content_type.unwrap().contains("application/fhir+json"));

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(json["resourceType"], "Patient");
    assert!(json["id"].is_string());
    assert_eq!(json["gender"], "male");
}

#[tokio::test]
async fn test_fhir_backend_update_patient() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let patient = json!({
        "resourceType": "Patient",
        "name": [{
            "family": "UpdatedName",
            "given": ["Updated"]
        }],
        "gender": "female"
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/Patient/456")
                .method("PUT")
                .header("content-type", "application/fhir+json")
                .body(Body::from(serde_json::to_vec(&patient).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(json["id"], "456");
    assert_eq!(json["resourceType"], "Patient");
}

#[tokio::test]
async fn test_fhir_backend_delete_patient() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/Patient/789")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_fhir_backend_search_with_params() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/Observation?patient=123&category=laboratory")
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

    assert_eq!(json["resourceType"], "Bundle");
    assert!(json["search_params"].is_object());
    assert_eq!(json["search_params"]["patient"], "123");
}

#[tokio::test]
async fn test_fhir_backend_content_type_preserved() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/_status")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Verify FHIR-specific content type is present
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    
    assert!(content_type.is_some());
    assert!(content_type.unwrap().contains("application/fhir+json"));

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(json["status"], "operational");
    assert_eq!(json["version"], "R4");
}

#[tokio::test]
async fn test_fhir_backend_accepts_fhir_json() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/Patient/123")
                .method("GET")
                .header("accept", "application/fhir+json")
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

    assert_eq!(json["resourceType"], "Patient");
}

#[tokio::test]
async fn test_fhir_backend_normalizes_json() {
    let (backend_url, _handle) = build_mock_fhir_server().await;
    let app = build_test_router(&backend_url).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/fhir/Patient/123")
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
    
    // Verify response is valid JSON that can be parsed
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json should parse");
    
    // Verify FHIR resource structure
    assert!(json.is_object());
    assert!(json.get("resourceType").is_some());
    assert!(json.get("id").is_some());
}

#[tokio::test]
async fn test_fhir_backend_missing_base_url_fails_validation() {
    let toml = r#"
        [proxy]
        id = "fhir-backend-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8082

        [pipelines.core]
        description = "FHIR pipeline"
        networks = ["default"]
        endpoints = ["fhir_endpoint"]
        backends = ["fhir_backend"]
        middleware = []

        [endpoints.fhir_endpoint]
        service = "fhir"
        [endpoints.fhir_endpoint.options]
        path_prefix = "/fhir"

        [backends.fhir_backend]
        service = "fhir"
        [backends.fhir_backend.options]
        # Missing base_url - should cause runtime error when backend is called

        [services.fhir]
        module = ""
    "#;

    let _ = std::fs::create_dir_all("../../tmp");
    let cfg = load_config_from_str(toml).expect("config should parse");
    let router = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Request should fail because backend lacks base_url
    let response = router
        .oneshot(
            Request::builder()
                .uri("/fhir/Patient/123")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // Should get an error status (502 Bad Gateway when backend config is invalid)
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}
