//! Integration tests for automatically injected ingress `mesh_auth`.
//!
//! The pipelines here do NOT list `mesh_auth`. A request that arrives through
//! an ingress of an enabled mesh must still present a valid mesh JWT, for
//! both ordinary and streaming pipelines. A request without one must get a
//! 401 and the backend must never see it.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::any;
use axum::Router;
use harmony::adapters::http::router::build_network_router;
use harmony::config::config::Config;
use harmony::models::backends::backends::Backend;
use harmony::models::endpoints::endpoint::Endpoint;
use harmony::models::mesh::config::{
    IngressEgressMode, Mesh, MeshAuthType, MeshIngress, MeshProtocol, MeshProvider,
};
use harmony::models::middleware::types::mesh_auth::MeshClaims;
use harmony::models::pipelines::config::Pipeline;
use harmony::models::services::services::ServiceConfig;
use harmony::pipeline::PipelineExecutor;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tower::ServiceExt;

const MESH_NAME: &str = "auto_mesh";
const MESH_SECRET: &str = "auto-auth-mesh-secret";
const PLAIN_HOST: &str = "plain.example.com";
const STREAM_HOST: &str = "stream.example.com";

/// Start an upstream that counts every request and answers with one NDJSON
/// line, which both the `http` and `http_streaming_backend` backends accept.
async fn start_counting_backend() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_for_handler = hits.clone();
    let app = Router::new().route(
        "/{*path}",
        any(move || {
            let hits = hits_for_handler.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                (
                    [(header::CONTENT_TYPE, "application/x-ndjson")],
                    "{\"backend\":\"reached\"}\n",
                )
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, hits)
}

fn http_endpoint() -> Endpoint {
    Endpoint {
        service: "http".to_string(),
        options: Some(HashMap::from([(
            "path_prefix".to_string(),
            serde_json::json!("/api"),
        )])),
        peer_ref: None,
        connection: None,
        authentication: None,
    }
}

fn backend(service: &str, base_url: &str) -> Backend {
    Backend {
        service: service.to_string(),
        target_ref: None,
        connection: None,
        authentication: None,
        timeout_secs: None,
        max_retries: None,
        options: Some(HashMap::from([(
            "base_url".to_string(),
            serde_json::json!(base_url),
        )])),
    }
}

fn ingress(pipeline: &str, endpoint: &str, host: &str) -> MeshIngress {
    MeshIngress {
        id: None,
        ingress_type: MeshProtocol::Http,
        pipeline: pipeline.to_string(),
        mode: IngressEgressMode::Default,
        endpoint: Some(endpoint.to_string()),
        urls: vec![format!("http://{}/api", host)],
        description: None,
        enabled: true,
    }
}

/// One enabled mesh with two ingresses. `plain_pipeline` uses an `http`
/// backend and `stream_pipeline` uses an `http_streaming_backend`. Neither
/// pipeline lists `mesh_auth`.
fn mesh_config(backend_base_url: &str) -> Config {
    let mut config = Config::default();

    config.network.insert(
        "default".to_string(),
        harmony::models::network::config::NetworkConfig {
            enable_wireguard: false,
            interface: "wg0".to_string(),
            tcp_config: Some(harmony::models::network::config::TcpConfig {
                bind_address: "127.0.0.1".to_string(),
                bind_port: 8080,
                cert_path: None,
                key_path: None,
                force_https: false,
            }),
            http3: None,
        },
    );

    for service in ["http", "http_streaming_backend"] {
        config.services.insert(
            service.to_string(),
            ServiceConfig {
                id: None,
                module: "".to_string(),
            },
        );
    }

    config
        .endpoints
        .insert("plain_endpoint".to_string(), http_endpoint());
    config
        .endpoints
        .insert("stream_endpoint".to_string(), http_endpoint());
    config.backends.insert(
        "plain_upstream".to_string(),
        backend("http", backend_base_url),
    );
    config.backends.insert(
        "stream_upstream".to_string(),
        backend("http_streaming_backend", backend_base_url),
    );

    config.pipelines.insert(
        "plain_pipeline".to_string(),
        Pipeline {
            description: "Mesh ingress to an http backend".to_string(),
            networks: vec!["default".to_string()],
            endpoints: vec!["plain_endpoint".to_string()],
            backends: vec!["plain_upstream".to_string()],
            ..Default::default()
        },
    );
    config.pipelines.insert(
        "stream_pipeline".to_string(),
        Pipeline {
            description: "Mesh ingress to a streaming backend".to_string(),
            networks: vec!["default".to_string()],
            endpoints: vec!["stream_endpoint".to_string()],
            backends: vec!["stream_upstream".to_string()],
            ..Default::default()
        },
    );

    config.ingress.insert(
        "plain_ingress".to_string(),
        ingress("plain_pipeline", "plain_endpoint", PLAIN_HOST),
    );
    config.ingress.insert(
        "stream_ingress".to_string(),
        ingress("stream_pipeline", "stream_endpoint", STREAM_HOST),
    );

    config.mesh.insert(
        MESH_NAME.to_string(),
        Mesh {
            id: None,
            mesh_type: MeshProtocol::Http,
            provider: MeshProvider::Local,
            auth_type: MeshAuthType::Jwt,
            jwt_secret: Some(MESH_SECRET.to_string()),
            jwt_private_key_path: None,
            jwt_public_key_path: None,
            jwks_url: None,
            ingress: vec!["plain_ingress".to_string(), "stream_ingress".to_string()],
            egress: vec![],
            description: None,
            enabled: true,
        },
    );

    config
}

fn sign_token(secret: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let claims = MeshClaims {
        sub: Some("peer-proxy".to_string()),
        iss: MESH_NAME.to_string(),
        aud: None,
        iat: now,
        exp: now + 300,
        mesh_id: MESH_NAME.to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

/// Send one GET to the ingress on `host` and return the status and the
/// number of requests the backend received.
async fn send_through_ingress(host: &str, authorization: Option<String>) -> (StatusCode, usize) {
    let (backend_url, hits) = start_counting_backend().await;
    let config = Arc::new(mesh_config(&backend_url));
    harmony::globals::set_config(config.clone());
    let app = build_network_router(config, "default").await;

    let mut request = Request::builder()
        .uri("/api/resource")
        .method("GET")
        .header("Host", host);
    if let Some(value) = authorization {
        request = request.header("Authorization", value);
    }

    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .expect("router handled request");

    (response.status(), hits.load(Ordering::SeqCst))
}

#[test]
#[serial_test::serial]
fn stream_pipeline_is_a_streaming_pipeline() {
    let config = mesh_config("http://127.0.0.1:1");
    let pipeline = config.pipelines.get("stream_pipeline").unwrap();
    assert!(PipelineExecutor::is_streaming_pipeline(pipeline, &config));
    let pipeline = config.pipelines.get("plain_pipeline").unwrap();
    assert!(!PipelineExecutor::is_streaming_pipeline(pipeline, &config));
}

#[tokio::test]
#[serial_test::serial]
async fn plain_ingress_rejects_missing_token_before_backend() {
    let (status, hits) = send_through_ingress(PLAIN_HOST, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(hits, 0, "backend must not be called without a mesh token");
}

#[tokio::test]
#[serial_test::serial]
async fn plain_ingress_rejects_wrong_secret_before_backend() {
    let token = sign_token("not-the-mesh-secret");
    let (status, hits) = send_through_ingress(PLAIN_HOST, Some(format!("Bearer {}", token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(hits, 0, "backend must not be called with an invalid token");
}

#[tokio::test]
#[serial_test::serial]
async fn plain_ingress_accepts_valid_token() {
    let token = sign_token(MESH_SECRET);
    let (status, hits) = send_through_ingress(PLAIN_HOST, Some(format!("Bearer {}", token))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hits, 1, "backend should be called exactly once");
}

#[tokio::test]
#[serial_test::serial]
async fn streaming_ingress_rejects_missing_token_before_backend() {
    let (status, hits) = send_through_ingress(STREAM_HOST, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(hits, 0, "backend must not be called without a mesh token");
}

#[tokio::test]
#[serial_test::serial]
async fn streaming_ingress_rejects_wrong_secret_before_backend() {
    let token = sign_token("not-the-mesh-secret");
    let (status, hits) = send_through_ingress(STREAM_HOST, Some(format!("Bearer {}", token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(hits, 0, "backend must not be called with an invalid token");
}

#[tokio::test]
#[serial_test::serial]
async fn streaming_ingress_accepts_valid_token() {
    let token = sign_token(MESH_SECRET);
    let (status, hits) = send_through_ingress(STREAM_HOST, Some(format!("Bearer {}", token))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hits, 1, "backend should be called exactly once");
}
