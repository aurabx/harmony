use crate::adapters::http::common::{build_protocol_ctx, get_path_prefix};
use crate::adapters::http::router::map_pipeline_error_to_status;
use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::models::protocol::Protocol;
use crate::pipeline::PipelineExecutor;
use crate::router::route_config::RouteConfig;
use async_trait::async_trait;
use bytes::{Buf, Bytes};
use h3::server;
use h3_quinn::Connection as H3QuinnConnection;
use http::{Response as HttpResponse, StatusCode};
use matchit::Router as MatchRouter;
use quinn::{Endpoint, ServerConfig};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// How long shutdown waits for closed connections to drain, so peers see
/// the close instead of waiting out their idle timeout.
const HTTP3_CLOSE_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// How long a starting adapter keeps retrying a bind that fails because the
/// UDP port is still held by a previous endpoint that is closing.
const HTTP3_BIND_RETRY_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

/// HTTP/3 (QUIC) Protocol Adapter
///
/// This adapter will host a QUIC + HTTP/3 server for a given network. It uses
/// quinn + rustls for transport/TLS. Request handling will be wired into the
/// existing Harmony pipeline.
pub struct Http3Adapter {
    pub network_name: String,
    pub bind_addr: SocketAddr,
    pub cert_path: String,
    pub key_path: String,
}

impl Http3Adapter {
    pub fn new(network_name: String, bind_addr: SocketAddr, cert_path: String, key_path: String) -> Self {
        Self {
            network_name,
            bind_addr,
            cert_path,
            key_path,
        }
    }

    /// Build quinn ServerConfig from the adapter's TLS settings.
    fn build_server_config(&self) -> anyhow::Result<ServerConfig> {
        use quinn::crypto::rustls::QuicServerConfig;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        use rustls::ServerConfig as RustlsServerConfig;
        use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
        use std::fs::File;
        use std::io::BufReader;

        // Load certificate chain
        let mut cert_file = BufReader::new(File::open(&self.cert_path)?);
        let cert_chain: Vec<CertificateDer<'static>> = certs(&mut cert_file)
            .collect::<Result<_, _>>()?;

        if cert_chain.is_empty() {
            anyhow::bail!("no certificates found in {}", self.cert_path);
        }

        // Load private key (try PKCS#8 first, then RSA)
        let mut key_file = BufReader::new(File::open(&self.key_path)?);
        let mut keys: Vec<PrivateKeyDer<'static>> = pkcs8_private_keys(&mut key_file)
            .map(|r| r.map(|k| PrivateKeyDer::Pkcs8(k)))
            .collect::<Result<_, _>>()?;

        if keys.is_empty() {
            // Rewind and try RSA
            let mut key_file = BufReader::new(File::open(&self.key_path)?);
            keys = rsa_private_keys(&mut key_file)
                .map(|r| r.map(|k| PrivateKeyDer::Pkcs1(k)))
                .collect::<Result<_, _>>()?;
        }

        let key = keys
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no private keys found in {}", self.key_path))?;

        // Build rustls config
        let mut tls_config = RustlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)?;

        // Enable HTTP/3 ALPN
        tls_config.alpn_protocols = vec![b"h3".to_vec()];

        let quic_tls = QuicServerConfig::try_from(tls_config)?;
        let mut server_config = ServerConfig::with_crypto(Arc::new(quic_tls));
        // The peer address feeds ip_allow/ip_deny and rate limits. With
        // migration on, quinn makes a new client address current as soon as
        // one authenticated packet arrives from it, before PATH_CHALLENGE
        // validates it. A key-holding client could then forge an allowlisted
        // source IP for a request. With migration off, quinn drops packets
        // from any address other than the one the handshake validated. A
        // client whose IP changes must reconnect.
        server_config.migration(false);
        Ok(server_config)
    }

    /// Binds the QUIC endpoint, retrying while the port is still in use.
    ///
    /// A closing QUIC connection stays in quinn's endpoint for three probe
    /// timeouts, which grow with the peer's RTT and cannot be cut short. On a
    /// restart the previous endpoint can therefore still hold the UDP port.
    /// Returns None if binding fails for another reason, the retry window
    /// passes, or shutdown is requested.
    async fn bind_endpoint(
        &self,
        server_config: ServerConfig,
        shutdown: &CancellationToken,
    ) -> Option<Endpoint> {
        let give_up_at = tokio::time::Instant::now() + HTTP3_BIND_RETRY_WINDOW;
        let mut delay = std::time::Duration::from_millis(50);
        loop {
            let error = match Endpoint::server(server_config.clone(), self.bind_addr) {
                Ok(endpoint) => return Some(endpoint),
                Err(e) => e,
            };
            let retry = error.kind() == std::io::ErrorKind::AddrInUse
                && tokio::time::Instant::now() + delay < give_up_at;
            if !retry {
                tracing::error!(
                    "Failed to bind HTTP/3 endpoint for network '{}' on {}: {}",
                    self.network_name,
                    self.bind_addr,
                    error
                );
                return None;
            }
            tracing::debug!(
                "HTTP/3 port {} for network '{}' still in use; retrying in {:?}",
                self.bind_addr,
                self.network_name,
                delay
            );
            tokio::select! {
                _ = shutdown.cancelled() => return None,
                _ = tokio::time::sleep(delay) => {}
            }
            delay = (delay * 2).min(std::time::Duration::from_secs(1));
        }
    }

    async fn run_server(self, shutdown: CancellationToken) {
        let server_config = match self.build_server_config() {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::error!(
                    "Failed to build HTTP/3 server config for network '{}': {}",
                    self.network_name,
                    e
                );
                return;
            }
        };

        let Some(endpoint) = self.bind_endpoint(server_config, &shutdown).await else {
            return;
        };

        tracing::info!(
            "🚀 HTTP/3 adapter started for network '{}' on {}",
            self.network_name,
            self.bind_addr
        );

        // Connection tasks are tracked so shutdown can end them. Each holds a
        // quinn Connection, which keeps the UDP socket open until dropped.
        let mut connections = tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("HTTP/3 adapter for network '{}' shutting down", self.network_name);
                    break;
                }
                // Reap finished connection tasks so the set does not grow.
                Some(finished) = connections.join_next(), if !connections.is_empty() => {
                    if let Err(e) = finished {
                        tracing::error!("HTTP/3 connection task for network '{}' panicked: {}", self.network_name, e);
                    }
                }
                incoming = endpoint.accept() => {
                    match incoming {
                        Some(incoming_conn) => {
                            let network_name = self.network_name.clone();
                            connections.spawn(async move {
                                if let Err(e) = super::http3::handle_connection(incoming_conn, network_name).await {
                                    tracing::warn!("HTTP/3 connection error: {}", e);
                                }
                            });
                        }
                        None => {
                            // Endpoint is stopping
                            break;
                        }
                    }
                }
            }
        }

        // Stopping the accept loop does not end open connections. quinn keeps
        // the endpoint and its UDP socket alive while any connection is open
        // or any Connection handle exists. Open connections would keep serving
        // with the old TLS config after a cert rotation. Close them all, end
        // the tasks holding handles (a closed connection cannot answer a
        // request anyway), then give peers a short time to receive the close.
        endpoint.close(0u32.into(), b"server shutting down");
        connections.shutdown().await;
        if tokio::time::timeout(HTTP3_CLOSE_DRAIN_TIMEOUT, endpoint.wait_idle())
            .await
            .is_err()
        {
            tracing::warn!(
                "HTTP/3 adapter for network '{}': connections did not drain within {:?}",
                self.network_name,
                HTTP3_CLOSE_DRAIN_TIMEOUT
            );
        }

        tracing::info!("HTTP/3 adapter for network '{}' shut down", self.network_name);
    }
}

/// Handle an incoming QUIC connection and serve HTTP/3 requests.
///
/// This mirrors the HTTP adapter's behavior by:
/// - Resolving the pipeline + endpoint based on network, method, and path
/// - Building a ProtocolCtx equivalent to HttpAdapter::http_request_to_protocol_ctx
/// - Running the PipelineExecutor
/// - Mapping the resulting HTTP response back onto HTTP/3 frames
///
/// Each request reads the live global config once, so a zero-downtime
/// reload reaches the next request on an open connection.
pub async fn handle_connection(
    incoming: quinn::Incoming,
    network_name: String,
) -> anyhow::Result<()> {
    // Complete the QUIC handshake
    let connection = incoming.await?;
    // Kept so the peer address is read for each request. build_server_config
    // turns migration off, so this is the handshake-validated address. The
    // per-request read keeps that true if migration is ever turned back on:
    // a value cached at handshake would then go stale.
    let quic_conn = connection.clone();

    // Upgrade to HTTP/3 connection
    let mut h3_conn = server::builder()
        .build(H3QuinnConnection::new(connection))
        .await?;

    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                // Resolve the request (get headers)
                let (req, mut stream) = match resolver.resolve_request().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("HTTP/3 failed to resolve request: {}", e);
                        continue;
                    }
                };
                let peer_addr = quic_conn.remote_address();

                // Read request body from DATA frames
                let mut body_bytes: Vec<u8> = Vec::new();
                while let Some(mut data) = stream.recv_data().await? {
                    body_bytes.extend_from_slice(data.chunk());
                    data.advance(data.remaining());
                }

                let (parts, _) = req.into_parts();
                let method = parts.method;
                let uri = parts.uri;
                let headers = parts.headers;

                let path = uri.path().to_string();
                let normalized_path = if path != "/" && path.ends_with('/') {
                    path.trim_end_matches('/').to_string()
                } else {
                    path.clone()
                };

                let config = match crate::adapters::http::router::live_config() {
                    Ok(config) => config,
                    Err(status) => {
                        let resp = HttpResponse::builder().status(status).body(())?;
                        stream.send_response(resp).await?;
                        stream.finish().await?;
                        continue;
                    }
                };

                // Find matching pipeline + endpoint based on network, path and method
                let mut selected: Option<(String, String)> = None;
                for (pipeline_name, pipeline) in &config.pipelines {
                    if !pipeline.networks.contains(&network_name) {
                        continue;
                    }

                    for endpoint_name in &pipeline.endpoints {
                        let endpoint = match config.endpoints.get(endpoint_name) {
                            Some(e) => e,
                            None => {
                                tracing::warn!(
                                    "Endpoint '{}' referenced in pipeline '{}' not found in config (HTTP/3)",
                                    endpoint_name,
                                    pipeline_name
                                );
                                continue;
                            }
                        };

                        let service = match endpoint.resolve_service() {
                            Ok(svc) => svc,
                            Err(err) => {
                                tracing::error!(
                                    "Failed to resolve service '{}' for endpoint '{}' in pipeline '{}' (HTTP/3): {}",
                                    endpoint.service,
                                    endpoint_name,
                                    pipeline_name,
                                    err
                                );
                                continue;
                            }
                        };

                        let opts_map: HashMap<String, serde_json::Value> =
                            endpoint.options.clone().unwrap_or_default();

                        let route_configs: Vec<RouteConfig> = service.build_router(&opts_map);

                        for route_config in route_configs {
                            if !route_config.methods.contains(&method) {
                                continue;
                            }

                            let mut router = MatchRouter::new();
                            if let Err(e) = router.insert(&route_config.path, ()) {
                                tracing::error!(
                                    "Failed to insert HTTP/3 route pattern '{}' for endpoint '{}' in pipeline '{}': {}",
                                    route_config.path,
                                    endpoint_name,
                                    pipeline_name,
                                    e
                                );
                                continue;
                            }

                            if router.at(&normalized_path).is_ok() {
                                selected = Some((endpoint_name.clone(), pipeline_name.clone()));
                                break;
                            }
                        }

                        if selected.is_some() {
                            break;
                        }
                    }

                    if selected.is_some() {
                        break;
                    }
                }

                // If no route matched, return 404
                let (endpoint_name, pipeline_name) = match selected {
                    Some(v) => v,
                    None => {
                        tracing::warn!(
                            "HTTP/3 request {} {} on network '{}' did not match any endpoint pipeline",
                            method,
                            path,
                            network_name
                        );

                        let resp = HttpResponse::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(())?;
                        stream.send_response(resp).await?;
                        stream.finish().await?;
                        continue;
                    }
                };

                let endpoint = match config.endpoints.get(&endpoint_name) {
                    Some(e) => e,
                    None => {
                        tracing::error!(
                            "Selected endpoint '{}' not found in config during HTTP/3 handling",
                            endpoint_name
                        );
                        let resp = HttpResponse::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(())?;
                        stream.send_response(resp).await?;
                        stream.finish().await?;
                        continue;
                    }
                };

                let pipeline = match config.pipelines.get(&pipeline_name) {
                    Some(p) => p,
                    None => {
                        tracing::error!(
                            "Selected pipeline '{}' not found in config during HTTP/3 handling",
                            pipeline_name
                        );
                        let resp = HttpResponse::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(())?;
                        stream.send_response(resp).await?;
                        stream.finish().await?;
                        continue;
                    }
                };

                let default_options = HashMap::new();
                let options: &HashMap<String, serde_json::Value> =
                    endpoint.options.as_ref().unwrap_or(&default_options);

                // Build ProtocolCtx using shared helper
                let path_prefix = get_path_prefix(options);
                // The QUIC peer address feeds remote_addr, which IP allowlists
                // and per-IP rate limits key on.
                let ctx = build_protocol_ctx(
                    &method,
                    &uri,
                    &headers,
                    body_bytes,
                    path_prefix,
                    "http3",
                    Some(peer_addr),
                );

                // Resolve service again (we dropped it earlier when leaving scope)
                let service = match endpoint.resolve_service() {
                    Ok(svc) => svc,
                    Err(err) => {
                        tracing::error!(
                            "Failed to resolve service for endpoint '{}' during HTTP/3 handling: {}",
                            endpoint_name,
                            err
                        );
                        let resp = HttpResponse::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(())?;
                        stream.send_response(resp).await?;
                        stream.finish().await?;
                        continue;
                    }
                };

                // Build envelope via service
                let envelope = match service
                    .build_protocol_envelope(ctx.clone(), options)
                    .await
                {
                    Ok(env) => env,
                    Err(err) => {
                        tracing::warn!(
                            "HTTP/3 build_protocol_envelope failed for endpoint '{}': {}",
                            endpoint_name,
                            err
                        );
                        let resp = HttpResponse::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .body(())?;
                        stream.send_response(resp).await?;
                        stream.finish().await?;
                        continue;
                    }
                };

                // Execute pipeline
                let response_envelope = match PipelineExecutor::execute(envelope, &pipeline_name, pipeline, &config, &ctx).await {
                    Ok(env) => env,
                    Err(err) => {
                        tracing::error!(
                            "HTTP/3 pipeline execution failed for endpoint '{}' in pipeline '{}': {}",
                            endpoint_name,
                            pipeline_name,
                            err
                        );
                        let status = map_pipeline_error_to_status(&err);
                        let resp = HttpResponse::builder().status(status).body(())?;
                        stream.send_response(resp).await?;
                        stream.finish().await?;
                        continue;
                    }
                };

                // Convert ResponseEnvelope → HTTP response via service
                let response = match service
                    .endpoint_outgoing_response(response_envelope, options)
                    .await
                {
                    Ok(resp) => resp,
                    Err(err) => {
                        tracing::error!(
                            "HTTP/3 endpoint_outgoing_response failed for endpoint '{}': {}",
                            endpoint_name,
                            err
                        );
                        let resp = HttpResponse::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(())?;
                        stream.send_response(resp).await?;
                        stream.finish().await?;
                        continue;
                    }
                };

                // Map axum::Response<Body> to HTTP/3 response frames
                let status = response.status();
                let resp_headers = response.headers().clone();
                let resp_body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;

                let mut builder = HttpResponse::builder().status(status);
                for (name, value) in resp_headers.iter() {
                    builder = builder.header(name, value);
                }

                let h3_response = builder.body(())?;
                stream.send_response(h3_response).await?;
                stream.send_data(Bytes::from(resp_body_bytes)).await?;
                stream.finish().await?;
            }
            Ok(None) => {
                // Connection cleanly closed
                break;
            }
            Err(err) => {
                // h3 0.0.8 ConnectionError doesn't have get_error_level;
                // treat all as connection-level for now
                tracing::warn!("HTTP/3 connection error: {}", err);
                break;
            }
        }
    }

    Ok(())
}

#[async_trait]
impl ProtocolAdapter for Http3Adapter {
    fn protocol(&self) -> Protocol {
        // HTTP/3 is treated as HTTP at the protocol layer
        Protocol::Http
    }

    fn from_network(
        network_name: String,
        network_config: &crate::models::network::config::NetworkConfig,
    ) -> Box<dyn ProtocolAdapter> {
        let http3 = network_config.http3.as_ref().unwrap_or_else(|| {
            panic!(
                "Http3Adapter requested for network '{}' but no [network.{}.http3] config was provided",
                network_name, network_name
            )
        });

        let bind_addr = format!("{}:{}", http3.bind_address, http3.bind_port)
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| {
                panic!(
                    "Invalid HTTP/3 bind address or port for network '{}': {}:{}",
                    network_name, http3.bind_address, http3.bind_port
                )
            });

        Box::new(Http3Adapter::new(
            network_name,
            bind_addr,
            http3.cert_path.clone(),
            http3.key_path.clone(),
        ))
    }

    /// `config` is unused: requests read the live global config instead.
    async fn start(
        &self,
        _config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let adapter = Http3Adapter {
            network_name: self.network_name.clone(),
            bind_addr: self.bind_addr,
            cert_path: self.cert_path.clone(),
            key_path: self.key_path.clone(),
        };

        Ok(tokio::spawn(async move {
            adapter.run_server(shutdown).await;
        }))
    }

    fn summary(&self) -> String {
        format!("Http3Adapter(network={}, bind={})", self.network_name, self.bind_addr)
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use std::time::Duration;

    /// Writes a self-signed cert and key for localhost into `dir` and returns
    /// (cert_path, key_path, cert_pem).
    fn write_test_cert(dir: &tempfile::TempDir) -> (String, String, String) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("generate self-signed cert");
        let cert_pem = cert.serialize_pem().expect("serialize cert");
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, &cert_pem).unwrap();
        std::fs::write(&key_path, cert.serialize_private_key_pem()).unwrap();
        (
            cert_path.to_string_lossy().into_owned(),
            key_path.to_string_lossy().into_owned(),
            cert_pem,
        )
    }

    fn client_endpoint(cert_pem: &str) -> Endpoint {
        use rustls::pki_types::CertificateDer;
        let mut roots = rustls::RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut cert_pem.as_bytes()) {
            roots.add(CertificateDer::from(cert.unwrap())).unwrap();
        }
        let mut tls = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        tls.alpn_protocols = vec![b"h3".to_vec()];
        let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap();
        let mut endpoint = Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic)));
        endpoint
    }

    /// Picks a free UDP port on localhost.
    fn free_udp_addr() -> SocketAddr {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.local_addr().unwrap()
    }

    /// Cancelling shutdown must end open QUIC connections, so none keep
    /// serving with the old TLS config, and must release the UDP port so a
    /// restart can bind it again.
    #[tokio::test]
    async fn shutdown_closes_open_connections_and_releases_port() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let dir = tempfile::TempDir::new().unwrap();
        let (cert_path, key_path, cert_pem) = write_test_cert(&dir);
        let bind_addr = free_udp_addr();

        let adapter = Http3Adapter::new("test".to_string(), bind_addr, cert_path, key_path);
        let shutdown = CancellationToken::new();
        let server = adapter
            .start(Arc::new(Config::default()), shutdown.clone())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = client_endpoint(&cert_pem);
        let connection = tokio::time::timeout(
            Duration::from_secs(5),
            client.connect(bind_addr, "localhost").unwrap(),
        )
        .await
        .expect("handshake timed out")
        .expect("handshake failed");

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("adapter task did not finish")
            .unwrap();

        // quinn's default idle timeout is 30 s, so a close within 3 s means
        // the server closed the connection.
        tokio::time::timeout(Duration::from_secs(3), connection.closed())
            .await
            .expect("connection still open after adapter shutdown");

        std::net::UdpSocket::bind(bind_addr).expect("UDP port still bound after shutdown");
    }

    /// On a restart the previous endpoint may still hold the UDP port while
    /// its connections finish closing. The new adapter must wait for the port
    /// instead of giving up, or HTTP/3 stays down until the next reload.
    #[tokio::test]
    async fn start_waits_for_a_port_still_held_by_a_closing_endpoint() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let dir = tempfile::TempDir::new().unwrap();
        let (cert_path, key_path, cert_pem) = write_test_cert(&dir);
        let holder = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let bind_addr = holder.local_addr().unwrap();

        let adapter = Http3Adapter::new("test".to_string(), bind_addr, cert_path, key_path);
        let shutdown = CancellationToken::new();
        let server = adapter
            .start(Arc::new(Config::default()), shutdown.clone())
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(holder);

        let client = client_endpoint(&cert_pem);
        let connection = tokio::time::timeout(
            Duration::from_secs(5),
            client.connect(bind_addr, "localhost").unwrap(),
        )
        .await
        .expect("adapter never bound the port after it was released")
        .expect("handshake failed");
        connection.close(0u32.into(), b"done");

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("adapter task did not finish")
            .unwrap();
    }
}
