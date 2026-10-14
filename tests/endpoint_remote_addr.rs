//! Every endpoint service copies the client address into the request envelope.
//!
//! The policies middleware reads `remote_addr` for IP allow and deny rules and
//! for rate-limit keys. An endpoint that drops it makes an `ip_allow` rule deny
//! every request, an `ip_deny` rule admit every request, and every client share
//! one rate-limit bucket.

use std::collections::HashMap;
use std::net::SocketAddr;

use harmony::adapters::http::common::build_protocol_ctx;
use harmony::management::ManagementEndpoint;
use harmony::models::envelope::command::{Command, Operation};
use harmony::models::services::types::dicom_scp::DicomScpEndpoint;
use harmony::models::services::types::dicomweb::DicomwebEndpoint;
use harmony::models::services::types::echo::EchoEndpoint;
use harmony::models::services::types::fhir::FhirEndpoint;
use harmony::models::services::types::http::HttpEndpoint;
use harmony::models::services::types::http_streaming::HttpStreamingEndpoint;
use harmony::models::services::types::jmix::JmixEndpoint;
use harmony::{Protocol, ProtocolCtx, ServiceType};
use http::{HeaderMap, Method, Uri};
use serde_json::Value;

const CLIENT: &str = "198.51.100.23:51234";

/// A GET as the HTTP/1.1 and HTTP/3 adapters record it, from `CLIENT`.
fn http_ctx() -> ProtocolCtx {
    let uri: Uri = "/any/path".parse().unwrap();
    let client: SocketAddr = CLIENT.parse().unwrap();
    build_protocol_ctx(
        &Method::GET,
        &uri,
        &HeaderMap::new(),
        Vec::new(),
        "/any",
        "http",
        Some(client),
    )
}

/// Every service that can serve an HTTP endpoint.
fn http_endpoints() -> Vec<(&'static str, Box<dyn ServiceType<ReqBody = Value>>)> {
    vec![
        ("http", Box::new(HttpEndpoint {})),
        ("fhir", Box::new(FhirEndpoint {})),
        ("dicomweb", Box::new(DicomwebEndpoint {})),
        ("jmix", Box::new(JmixEndpoint {})),
        ("echo", Box::new(EchoEndpoint {})),
        (
            "http_streaming",
            Box::new(HttpStreamingEndpoint {
                default_format: None,
                buffer_size: None,
                enable_compression: None,
            }),
        ),
        ("management", Box::new(ManagementEndpoint {})),
    ]
}

#[tokio::test]
async fn every_http_endpoint_keeps_the_client_address() {
    for (name, service) in http_endpoints() {
        let envelope = service
            .build_protocol_envelope(http_ctx(), &HashMap::new())
            .await
            .unwrap_or_else(|e| panic!("{} should build an envelope: {}", name, e));
        let metadata = &envelope.request_details.metadata;
        assert_eq!(
            metadata.get("remote_addr").map(String::as_str),
            Some("198.51.100.23"),
            "{} dropped remote_addr",
            name
        );
        assert_eq!(
            metadata.get("remote_port").map(String::as_str),
            Some("51234"),
            "{} dropped remote_port",
            name
        );
    }
}

/// The DIMSE adapter puts the association peer in `ctx.meta`. The `dicom_scp`
/// endpoint must carry it onto the envelope.
#[tokio::test]
async fn dicom_scp_endpoint_keeps_the_client_address() {
    let ctx = ProtocolCtx {
        protocol: Protocol::Dimse,
        payload: b"{}".to_vec(),
        meta: HashMap::from([
            ("remote_addr".to_string(), "10.20.30.40".to_string()),
            ("remote_port".to_string(), "40404".to_string()),
            ("protocol".to_string(), "dimse".to_string()),
        ]),
        attrs: serde_json::json!({}),
        command: Some(Command::new(Operation::Store)),
    };
    let endpoint = DicomScpEndpoint {
        local_aet: None,
        bind_addr: None,
        port: None,
        enable_echo: None,
        enable_find: None,
        enable_move: None,
        enable_get: None,
        enable_store: Some(true),
        storage_dir: None,
    };

    let envelope = endpoint
        .build_protocol_envelope(ctx, &HashMap::new())
        .await
        .expect("dicom_scp should build an envelope from a DIMSE context");
    let metadata = &envelope.request_details.metadata;
    assert_eq!(
        metadata.get("remote_addr").map(String::as_str),
        Some("10.20.30.40")
    );
    assert_eq!(
        metadata.get("remote_port").map(String::as_str),
        Some("40404")
    );
}

/// An `ip_allow` rule gates a FHIR endpoint through the HTTP router. FHIR
/// builds its envelope through the HTTP endpoint, so this fails if that
/// delegation ever stops carrying the client address.
#[tokio::test]
async fn ip_allowlist_gates_a_fhir_endpoint() {
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode};
    use harmony::config::config::Config;
    use harmony::router::build_network_router;
    use std::sync::Arc;
    use tower::ServiceExt;

    let config_toml = r#"
        [proxy]
        id = "fhir-ip-allowlist-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.fhir_gated]
        networks = ["default"]
        endpoints = ["fhir_api"]
        middleware = ["ip_gate"]
        backends = ["echo_backend"]

        [endpoints.fhir_api]
        service = "fhir"
        [endpoints.fhir_api.options]
        path_prefix = "/fhir"

        [backends.echo_backend]
        service = "echo"

        [middleware.ip_gate]
        type = "policies"
        [middleware.ip_gate.options]
        policies = ["ip_gate_policy"]

        [policies.ip_gate_policy]
        id = "ip_gate_policy"
        enabled = true
        rules = ["clinic_network"]

        [rules.clinic_network]
        id = "clinic_network"
        type = "ip_allow"
        weight = 100
        enabled = true
        [rules.clinic_network.options]
        ip_addresses = ["10.0.0.0/8"]

        [middleware_types.policies]
        module = ""

        [services.fhir]
        module = ""

        [services.echo]
        module = ""
    "#;

    let config: Config = toml::from_str(config_toml).unwrap();
    config.validate().unwrap();
    let config = Arc::new(config);

    harmony::globals::set_config(config.clone());
    let storage = harmony::storage::create_storage_backend(&config.storage)
        .expect("Failed to create storage backend");
    harmony::globals::set_storage(storage);

    let app = build_network_router(config, "default").await;

    let request_from = |peer: &str| {
        let addr: SocketAddr = peer.parse().expect("valid socket address");
        Request::builder()
            .uri("/fhir/Patient/123")
            .method("GET")
            .extension(ConnectInfo(addr))
            .body(Body::empty())
            .unwrap()
    };

    let inside = app
        .clone()
        .oneshot(request_from("10.1.2.3:40000"))
        .await
        .unwrap();
    assert_eq!(inside.status(), StatusCode::OK);

    let outside = app
        .oneshot(request_from("203.0.113.9:40000"))
        .await
        .unwrap();
    assert_eq!(outside.status(), StatusCode::FORBIDDEN);
}
