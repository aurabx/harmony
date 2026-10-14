//! Target authentication and `base_path` on the HTTP/3 backend paths.
//!
//! A recording HTTP/3 upstream captures the request target and every header,
//! including repeated ones. The tests prove that the `http3` service and the
//! HTTP/3 branches of the `http` and `fhir` services send the target's
//! credential, never the client's, and put `base_path` in front of the request
//! path, the way the HTTP/1.1 and HTTP/2 paths do.
//! Linear: AURA-2435, AURA-2433.
//!
//! The upstream sets no ALPN protocol because `Http3Client` offers none yet
//! (AURA-2453). Add `h3` here once the client sends it.

use harmony::models::envelope::envelope::{RequestEnvelope, TargetDetails};
use harmony::models::services::services::ServiceHandler;
use harmony::models::services::types::fhir::FhirEndpoint;
use harmony::models::services::types::http::HttpEndpoint;
use harmony::models::services::types::http3::Http3Backend;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

const CLIENT_AUTH: &str = "Basic Y2xpZW50OmNsaWVudC1wYXNzd29yZA==";
const TARGET_TOKEN: &str = "target-token-123";

static INSTALL_CRYPTO: Once = Once::new();

fn install_crypto_provider() {
    INSTALL_CRYPTO.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// One request as the upstream saw it.
#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    /// Path and query, as sent.
    target: String,
    /// Lowercased name and value, in arrival order. Repeats are kept.
    headers: Vec<(String, String)>,
}

impl Recorded {
    fn values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

struct Http3Upstream {
    port: u16,
    ca_cert_path: String,
    recorded: Arc<Mutex<Vec<Recorded>>>,
    endpoint: quinn::Endpoint,
    _cert_dir: tempfile::TempDir,
}

impl Http3Upstream {
    fn only_request(&self) -> Recorded {
        let all = self.recorded.lock().unwrap();
        assert_eq!(
            all.len(),
            1,
            "expected exactly one upstream request, got {:?}",
            *all
        );
        all[0].clone()
    }
}

impl Drop for Http3Upstream {
    fn drop(&mut self) {
        self.endpoint.close(0u32.into(), b"test complete");
    }
}

async fn start_recording_upstream() -> Http3Upstream {
    install_crypto_provider();

    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("failed to generate self-signed cert");
    let cert_pem = cert.serialize_pem().expect("failed to serialize cert");
    let key_pem = cert.serialize_private_key_pem();

    // The project keeps scratch files under ./tmp, not the system temp directory.
    std::fs::create_dir_all("./tmp").expect("failed to create ./tmp");
    let cert_dir = tempfile::tempdir_in("./tmp").expect("failed to create cert dir");
    let ca_cert_path = cert_dir.path().join("ca.pem");
    std::fs::write(&ca_cert_path, &cert_pem).expect("failed to write CA cert");

    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_pem.as_bytes())
            .collect::<Result<_, _>>()
            .expect("failed to parse cert PEM");
    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .expect("failed to read key PEM")
        .expect("no key found in PEM");
    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("failed to build TLS config");
    let quic_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
        .expect("failed to create QUIC server config");
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_config));

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let endpoint = quinn::Endpoint::server(server_config, addr).expect("failed to start upstream");
    let port = endpoint.local_addr().expect("no local addr").port();

    let recorded = Arc::new(Mutex::new(Vec::new()));
    let accept_endpoint = endpoint.clone();
    let sink = recorded.clone();
    tokio::spawn(async move {
        while let Some(incoming) = accept_endpoint.accept().await {
            let sink = sink.clone();
            tokio::spawn(async move {
                if let Ok(connection) = incoming.await {
                    if let Err(e) = serve_connection(connection, sink).await {
                        eprintln!("recording upstream connection ended: {}", e);
                    }
                }
            });
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    Http3Upstream {
        port,
        ca_cert_path: ca_cert_path.to_string_lossy().to_string(),
        recorded,
        endpoint,
        _cert_dir: cert_dir,
    }
}

async fn serve_connection(
    connection: quinn::Connection,
    sink: Arc<Mutex<Vec<Recorded>>>,
) -> anyhow::Result<()> {
    use bytes::{Buf, Bytes};

    let mut h3 = h3::server::Connection::new(h3_quinn::Connection::new(connection)).await?;
    while let Some(resolver) = h3.accept().await? {
        let (request, mut stream) = resolver.resolve_request().await?;
        while let Some(mut chunk) = stream.recv_data().await? {
            chunk.advance(chunk.remaining());
        }

        sink.lock().unwrap().push(Recorded {
            method: request.method().as_str().to_string(),
            target: request
                .uri()
                .path_and_query()
                .map(|pq| pq.as_str().to_string())
                .unwrap_or_default(),
            headers: request
                .headers()
                .iter()
                .map(|(n, v)| {
                    (
                        n.as_str().to_string(),
                        v.to_str().unwrap_or("<non-utf8>").to_string(),
                    )
                })
                .collect(),
        });

        let response = http::Response::builder()
            .status(http::StatusCode::OK)
            .header("content-type", "application/json")
            .body(())?;
        stream.send_response(response).await?;
        stream
            .send_data(Bytes::from_static(b"{\"ok\":true}"))
            .await?;
        stream.finish().await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Service {
    Http3,
    Http,
    Fhir,
}

const ALL_SERVICES: [Service; 3] = [Service::Http3, Service::Http, Service::Fhir];

fn authentication_def(method: &str, options: Value) -> Value {
    json!({ "id": "test-auth", "method": method, "options": options })
}

/// Backend options as the config resolver would build them for `service`.
fn backend_options(
    service: Service,
    upstream: &Http3Upstream,
    base_path: Option<&str>,
    auth: Option<Value>,
) -> HashMap<String, Value> {
    let mut options = HashMap::new();
    match service {
        Service::Http3 => {
            options.insert("host".to_string(), json!("127.0.0.1"));
            options.insert("port".to_string(), json!(upstream.port));
            options.insert("ca_cert_path".to_string(), json!(upstream.ca_cert_path));
            if let Some(base_path) = base_path {
                options.insert("base_path".to_string(), json!(base_path));
            }
        }
        Service::Http | Service::Fhir => {
            let mut connection = json!({
                "host": "127.0.0.1",
                "port": upstream.port,
                "protocol": "h3",
                "ca_cert_path": upstream.ca_cert_path,
            });
            if let Some(base_path) = base_path {
                connection["base_path"] = json!(base_path);
            }
            options.insert("connection".to_string(), connection);
        }
    }
    if let Some(auth) = auth {
        options.insert("authentication_def".to_string(), auth);
    }
    options
}

/// A GET /data.json from a client that sent its own Authorization header.
///
/// Like the pipeline executor, it sets target details whose base URL already
/// includes the connection's `base_path`.
fn client_request(
    upstream: &Http3Upstream,
    base_path: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> RequestEnvelope<Vec<u8>> {
    let mut builder = RequestEnvelope::builder()
        .method("GET")
        .uri("/data.json")
        .header("Authorization", CLIENT_AUTH)
        .metadata_entry("path", "/data.json")
        .original_data(Vec::new());
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let mut envelope = builder.build().expect("failed to build envelope");

    let base_url = format!(
        "https://127.0.0.1:{}{}",
        upstream.port,
        base_path
            .map(|p| format!("/{}", p.trim_matches('/')))
            .unwrap_or_default()
    );
    let mut target = TargetDetails::from_request_details(base_url, &envelope.request_details);
    target.uri = "/data.json".to_string();
    envelope.target_details = Some(target);
    envelope
}

async fn send(
    service: Service,
    envelope: RequestEnvelope<Vec<u8>>,
    options: &HashMap<String, Value>,
) -> u16 {
    let result = match service {
        Service::Http3 => {
            Http3Backend::default()
                .backend_outgoing_request(envelope, options)
                .await
        }
        Service::Http => {
            HttpEndpoint {}
                .backend_outgoing_request(envelope, options)
                .await
        }
        Service::Fhir => {
            FhirEndpoint {}
                .backend_outgoing_request(envelope, options)
                .await
        }
    };
    result
        .unwrap_or_else(|e| panic!("{:?} backend request failed: {}", service, e))
        .response_details
        .status
}

#[tokio::test]
async fn bearer_target_auth_replaces_client_authorization_on_every_http3_path() {
    for service in ALL_SERVICES {
        let upstream = start_recording_upstream().await;
        let options = backend_options(
            service,
            &upstream,
            Some("/api"),
            Some(authentication_def(
                "bearer",
                json!({ "token": TARGET_TOKEN }),
            )),
        );

        let status = send(
            service,
            client_request(&upstream, Some("/api"), &[]),
            &options,
        )
        .await;

        assert_eq!(status, 200, "{:?}", service);
        let seen = upstream.only_request();
        assert_eq!(
            seen.values("authorization"),
            vec![format!("Bearer {}", TARGET_TOKEN)],
            "{:?}: upstream must get exactly the target's credential",
            service
        );
    }
}

#[tokio::test]
async fn basic_target_auth_is_sent_on_every_http3_path() {
    for service in ALL_SERVICES {
        let upstream = start_recording_upstream().await;
        let options = backend_options(
            service,
            &upstream,
            None,
            Some(authentication_def(
                "basic",
                json!({ "username": "svc", "password": "pw" }),
            )),
        );

        send(service, client_request(&upstream, None, &[]), &options).await;

        // base64("svc:pw")
        assert_eq!(
            upstream.only_request().values("authorization"),
            vec!["Basic c3ZjOnB3"],
            "{:?}",
            service
        );
    }
}

#[tokio::test]
async fn api_key_target_auth_is_sent_and_replaces_client_header_of_the_same_name() {
    for service in ALL_SERVICES {
        let upstream = start_recording_upstream().await;
        let options = backend_options(
            service,
            &upstream,
            None,
            Some(authentication_def(
                "api_key",
                json!({ "api_key": "target-key", "header_name": "X-Api-Token" }),
            )),
        );
        // The client sends the same header under another letter case.
        let envelope = client_request(&upstream, None, &[("x-api-token", "client-key")]);

        send(service, envelope, &options).await;

        let seen = upstream.only_request();
        assert_eq!(
            seen.values("x-api-token"),
            vec!["target-key"],
            "{:?}",
            service
        );
        // The client's Authorization is not forwarded by default, and the
        // target does not set one.
        assert!(seen.values("authorization").is_empty(), "{:?}", service);
    }
}

#[tokio::test]
async fn target_credential_wins_when_forward_authorization_is_enabled() {
    for service in ALL_SERVICES {
        let upstream = start_recording_upstream().await;
        let mut options = backend_options(
            service,
            &upstream,
            None,
            Some(authentication_def(
                "bearer",
                json!({ "token": TARGET_TOKEN }),
            )),
        );
        options.insert("forward_authorization".to_string(), json!(true));

        send(service, client_request(&upstream, None, &[]), &options).await;

        assert_eq!(
            upstream.only_request().values("authorization"),
            vec![format!("Bearer {}", TARGET_TOKEN)],
            "{:?}",
            service
        );
    }
}

#[tokio::test]
async fn backend_without_target_auth_sends_no_credential() {
    for service in ALL_SERVICES {
        let upstream = start_recording_upstream().await;
        let options = backend_options(service, &upstream, None, None);

        send(service, client_request(&upstream, None, &[]), &options).await;

        assert!(
            upstream.only_request().values("authorization").is_empty(),
            "{:?}: the client's credential must not reach the upstream",
            service
        );
    }
}

#[tokio::test]
async fn base_path_is_prepended_to_the_request_path_on_every_http3_path() {
    for service in ALL_SERVICES {
        // Written with and without slashes: both must join to /api/v1/data.json.
        for base_path in ["/api/v1", "api/v1/"] {
            let upstream = start_recording_upstream().await;
            let options = backend_options(service, &upstream, Some(base_path), None);

            send(
                service,
                client_request(&upstream, Some(base_path), &[]),
                &options,
            )
            .await;

            let seen = upstream.only_request();
            assert_eq!(seen.method, "GET");
            assert_eq!(
                seen.target, "/api/v1/data.json",
                "{:?} base_path={:?}",
                service, base_path
            );
        }
    }
}

#[tokio::test]
async fn request_path_is_unchanged_without_base_path() {
    for service in ALL_SERVICES {
        let upstream = start_recording_upstream().await;
        let options = backend_options(service, &upstream, None, None);

        send(service, client_request(&upstream, None, &[]), &options).await;

        assert_eq!(
            upstream.only_request().target,
            "/data.json",
            "{:?}",
            service
        );
    }
}

#[tokio::test]
async fn http3_service_applies_base_path_and_auth_without_executor_target_details() {
    // The fallback path: no target details on the envelope, so the service builds them.
    let upstream = start_recording_upstream().await;
    let options = backend_options(
        Service::Http3,
        &upstream,
        Some("/api"),
        Some(authentication_def(
            "bearer",
            json!({ "token": TARGET_TOKEN }),
        )),
    );
    let mut envelope = client_request(&upstream, Some("/api"), &[]);
    envelope.target_details = None;

    send(Service::Http3, envelope, &options).await;

    let seen = upstream.only_request();
    assert_eq!(seen.target, "/api/data.json");
    assert_eq!(
        seen.values("authorization"),
        vec![format!("Bearer {}", TARGET_TOKEN)]
    );
}
