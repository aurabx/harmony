//! Integration tests for ingress `mesh_auth` through the HTTP router.
//!
//! These prove that an ingress mesh JWT is checked before the backend is
//! called. A request with a missing or invalid token must get a 401 and the
//! backend must never see it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use harmony::config::config::{Config, ConfigError};
use harmony::models::middleware::types::mesh_auth::MeshClaims;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tower::ServiceExt;

const MESH_NAME: &str = "test_mesh";
const MESH_SECRET: &str = "mesh-ingress-test-secret";

/// Start an upstream server that counts every request it receives.
async fn start_counting_backend() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_for_handler = hits.clone();
    let app = Router::new().route(
        "/{*path}",
        any(move || {
            let hits = hits_for_handler.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                axum::Json(serde_json::json!({ "backend": "reached" }))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, hits, handle)
}

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

/// Pipeline with an explicitly configured ingress `mesh_auth` in front of an
/// HTTP backend.
fn mesh_ingress_config(backend_base_url: &str) -> String {
    format!(
        r#"
        [proxy]
        id = "mesh-auth-ingress-test"
        log_level = "info"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8082

        [pipelines.mesh_in]
        description = "Ingress mesh_auth in front of an HTTP backend"
        networks = ["default"]
        endpoints = ["mesh_endpoint"]
        backends = ["upstream"]
        middleware = ["mesh_in_auth"]

        [middleware.mesh_in_auth]
        type = "mesh_auth"
        [middleware.mesh_in_auth.options]
        mesh_name = "{mesh_name}"
        provider = "local"
        direction = "ingress"
        jwt_secret = "{secret}"

        [endpoints.mesh_endpoint]
        service = "http"
        [endpoints.mesh_endpoint.options]
        path_prefix = "/mesh"

        [backends.upstream]
        service = "http"
        [backends.upstream.options]
        base_url = "{backend}"

        [services.http]
        module = ""

        [middleware_types.mesh_auth]
        module = ""
    "#,
        mesh_name = MESH_NAME,
        secret = MESH_SECRET,
        backend = backend_base_url,
    )
}

fn sign_token(secret: &str, mesh_id: &str) -> String {
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
        mesh_id: mesh_id.to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

/// Send one GET through a fresh router and return the status and backend hit count.
async fn send_through_mesh_ingress(authorization: Option<String>) -> (StatusCode, usize) {
    let (backend_url, hits, _handle) = start_counting_backend().await;
    let config = load_config_from_str(&mesh_ingress_config(&backend_url)).expect("valid config");
    let config = Arc::new(config);
    harmony::globals::set_config(config.clone());
    let app = harmony::router::build_network_router(config, "default").await;

    let mut request = Request::builder().uri("/mesh/resource").method("GET");
    if let Some(value) = authorization {
        request = request.header("Authorization", value);
    }

    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .expect("router handled request");

    (response.status(), hits.load(Ordering::SeqCst))
}

#[tokio::test]
#[serial_test::serial]
async fn missing_mesh_token_is_rejected_before_backend() {
    let (status, hits) = send_through_mesh_ingress(None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(hits, 0, "backend must not be called without a mesh token");
}

#[tokio::test]
#[serial_test::serial]
async fn garbage_mesh_token_is_rejected_before_backend() {
    let (status, hits) = send_through_mesh_ingress(Some("Bearer not-a-jwt".to_string())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(hits, 0, "backend must not be called with a garbage token");
}

#[tokio::test]
#[serial_test::serial]
async fn wrong_secret_mesh_token_is_rejected_before_backend() {
    let token = sign_token("not-the-mesh-secret", MESH_NAME);
    let (status, hits) = send_through_mesh_ingress(Some(format!("Bearer {}", token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        hits, 0,
        "backend must not be called with a token signed by another key"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn mesh_id_mismatch_is_rejected_before_backend() {
    let token = sign_token(MESH_SECRET, "some_other_mesh");
    let (status, hits) = send_through_mesh_ingress(Some(format!("Bearer {}", token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        hits, 0,
        "backend must not be called for a token from another mesh"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn valid_mesh_token_reaches_backend() {
    let token = sign_token(MESH_SECRET, MESH_NAME);
    let (status, hits) = send_through_mesh_ingress(Some(format!("Bearer {}", token))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        hits, 1,
        "backend should be called exactly once for a valid token"
    );
}
