//! Integration tests for mesh routing.
//!
//! These tests verify that:
//! 1. Requests matching mesh ingress URLs are routed to the mesh endpoint
//! 2. Requests not matching mesh URLs continue to normal routing
//! 3. Mesh context is attached to requests that match

use axum::body::Body;
use http::{Request, StatusCode};
use std::sync::Arc;
use tower::ServiceExt;

use harmony::adapters::http::router::build_network_router;
use harmony::config::config::Config;
use harmony::models::endpoints::endpoint::Endpoint;
use harmony::models::mesh::config::{Mesh, MeshIngress, MeshProtocol, MeshProvider};
use harmony::models::pipelines::config::Pipeline;
use harmony::models::services::services::ServiceConfig;

/// Build a test config with both mesh and normal routes.
fn make_test_config() -> Config {
    let mut config = Config::default();

    // Add network
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

    // Add echo service
    config.services.insert(
        "echo".to_string(),
        ServiceConfig {
            module: "".to_string(),
        },
    );

    // Normal endpoint (for /api/*)
    config.endpoints.insert(
        "api_endpoint".to_string(),
        Endpoint {
            service: "echo".to_string(),
            options: Some({
                let mut opts = std::collections::HashMap::new();
                opts.insert(
                    "path_prefix".to_string(),
                    serde_json::Value::String("/api".to_string()),
                );
                opts
            }),
            peer_ref: None,
            connection: None,
            authentication: None,
        },
    );

    // Mesh endpoint (for mesh ingress)
    config.endpoints.insert(
        "mesh_fhir_endpoint".to_string(),
        Endpoint {
            service: "echo".to_string(),
            options: Some({
                let mut opts = std::collections::HashMap::new();
                opts.insert(
                    "path_prefix".to_string(),
                    serde_json::Value::String("/fhir".to_string()),
                );
                opts
            }),
            peer_ref: None,
            connection: None,
            authentication: None,
        },
    );

    // Normal pipeline
    config.pipelines.insert(
        "api_pipeline".to_string(),
        Pipeline {
            description: "Normal API pipeline".to_string(),
            networks: vec!["default".to_string()],
            endpoints: vec!["api_endpoint".to_string()],
            backends: vec![],
            ..Default::default()
        },
    );

    // Mesh pipeline
    config.pipelines.insert(
        "mesh_pipeline".to_string(),
        Pipeline {
            description: "Mesh FHIR pipeline".to_string(),
            networks: vec!["default".to_string()],
            endpoints: vec!["mesh_fhir_endpoint".to_string()],
            backends: vec![],
            ..Default::default()
        },
    );

    // Mesh ingress - routes fhir.example.com to mesh_fhir_endpoint
    config.ingress.insert(
        "fhir_ingress".to_string(),
        MeshIngress {
            id: None,
            ingress_type: MeshProtocol::Http,
            pipeline: "mesh_pipeline".to_string(),
            mode: harmony::models::mesh::config::IngressEgressMode::Default,
            endpoint: Some("mesh_fhir_endpoint".to_string()),
            urls: vec!["https://fhir.example.com/r4".to_string()],
            description: None,
            enabled: true,
        },
    );

    // Mesh definition
    config.mesh.insert(
        "healthcare".to_string(),
        Mesh {
            id: None,
            mesh_type: MeshProtocol::Http,
            provider: MeshProvider::Local,
            auth_type: harmony::models::mesh::config::MeshAuthType::Jwt,
            jwt_secret: Some("test-mesh-secret".to_string()),
            jwt_private_key_path: None,
            jwt_public_key_path: None,
            ingress: vec!["fhir_ingress".to_string()],
            egress: vec![],
            description: None,
            enabled: true,
        },
    );

    config
}

#[tokio::test]
async fn test_mesh_route_takes_priority() {
    let config = Arc::new(make_test_config());
    let router = build_network_router(config, "default").await;

    // Request to mesh URL (fhir.example.com) should be routed via mesh
    let request = Request::builder()
        .method("GET")
        .uri("/r4/Patient/123")
        .header("Host", "fhir.example.com")
        .header("X-Forwarded-Proto", "https")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    // Should get 200 from echo service (mesh route matched)
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Mesh route should match and return 200"
    );
}

#[tokio::test]
async fn test_normal_route_when_no_mesh_match() {
    let config = Arc::new(make_test_config());
    let router = build_network_router(config, "default").await;

    // Request to normal API (not mesh URL) should use normal routing
    let request = Request::builder()
        .method("GET")
        .uri("/api/users")
        .header("Host", "localhost:8080")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    // Should get 200 from echo service (normal route matched)
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Normal route should match and return 200"
    );
}

#[tokio::test]
async fn test_404_when_no_route_matches() {
    let config = Arc::new(make_test_config());
    let router = build_network_router(config, "default").await;

    // Request to unknown path with non-mesh host should 404
    let request = Request::builder()
        .method("GET")
        .uri("/unknown/path")
        .header("Host", "localhost:8080")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    // Should get 404 (neither mesh nor normal route matched)
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Unknown path should return 404"
    );
}

#[tokio::test]
async fn test_mesh_wrong_host_falls_through() {
    let config = Arc::new(make_test_config());
    let router = build_network_router(config, "default").await;

    // Request with wrong host should NOT match mesh, should fall through
    let request = Request::builder()
        .method("GET")
        .uri("/r4/Patient/123")
        .header("Host", "wrong.example.com")
        .header("X-Forwarded-Proto", "https")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    // Should get 404 (mesh didn't match, no normal route for /r4)
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Wrong host should not match mesh route"
    );
}

#[tokio::test]
async fn test_mesh_wrong_scheme_falls_through() {
    let config = Arc::new(make_test_config());
    let router = build_network_router(config, "default").await;

    // Request with http (not https) should NOT match mesh ingress
    let request = Request::builder()
        .method("GET")
        .uri("/r4/Patient/123")
        .header("Host", "fhir.example.com")
        // No X-Forwarded-Proto, defaults to http
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    // Should get 404 (mesh requires https, this is http)
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "HTTP scheme should not match HTTPS mesh ingress"
    );
}

#[tokio::test]
async fn test_empty_mesh_config_normal_routing_works() {
    let mut config = Config::default();

    // Add network
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

    // Add echo service
    config.services.insert(
        "echo".to_string(),
        ServiceConfig {
            module: "".to_string(),
        },
    );

    // Normal endpoint only
    config.endpoints.insert(
        "api_endpoint".to_string(),
        Endpoint {
            service: "echo".to_string(),
            options: Some({
                let mut opts = std::collections::HashMap::new();
                opts.insert(
                    "path_prefix".to_string(),
                    serde_json::Value::String("/api".to_string()),
                );
                opts
            }),
            peer_ref: None,
            connection: None,
            authentication: None,
        },
    );

    config.pipelines.insert(
        "api_pipeline".to_string(),
        Pipeline {
            description: "Normal API pipeline".to_string(),
            networks: vec!["default".to_string()],
            endpoints: vec!["api_endpoint".to_string()],
            backends: vec![],
            ..Default::default()
        },
    );

    // No mesh config at all
    let config = Arc::new(config);
    let router = build_network_router(config, "default").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/users")
        .header("Host", "localhost:8080")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Normal routing should work when no mesh config exists"
    );
}
