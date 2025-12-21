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
        Ok(ServerConfig::with_crypto(Arc::new(quic_tls)))
    }

    async fn run_server(self, config: Arc<Config>, shutdown: CancellationToken) {
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

        let endpoint = match Endpoint::server(server_config, self.bind_addr) {
            Ok(ep) => ep,
            Err(e) => {
                tracing::error!(
                    "Failed to bind HTTP/3 endpoint for network '{}' on {}: {}",
                    self.network_name,
                    self.bind_addr,
                    e
                );
                return;
            }
        };

        tracing::info!(
            "🚀 HTTP/3 adapter started for network '{}' on {}",
            self.network_name,
            self.bind_addr
        );

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("HTTP/3 adapter for network '{}' shutting down", self.network_name);
                    break;
                }
                incoming = endpoint.accept() => {
                    match incoming {
                        Some(incoming_conn) => {
                            let config = config.clone();
                            let network_name = self.network_name.clone();
                            tokio::spawn(async move {
                                if let Err(e) = super::http3::handle_connection(incoming_conn, config, network_name).await {
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
pub async fn handle_connection(
    incoming: quinn::Incoming,
    config: Arc<Config>,
    network_name: String,
) -> anyhow::Result<()> {
    // Complete the QUIC handshake
    let connection = incoming.await?;

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
                let ctx = build_protocol_ctx(
                    &method,
                    &uri,
                    &headers,
                    body_bytes,
                    path_prefix,
                    "http3",
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
                let response_envelope = match PipelineExecutor::execute(envelope, pipeline, &config, &ctx).await {
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

    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let adapter = Http3Adapter {
            network_name: self.network_name.clone(),
            bind_addr: self.bind_addr,
            cert_path: self.cert_path.clone(),
            key_path: self.key_path.clone(),
        };

        Ok(tokio::spawn(async move {
            adapter.run_server(config, shutdown).await;
        }))
    }

    fn summary(&self) -> String {
        format!("Http3Adapter(network={}, bind={})", self.network_name, self.bind_addr)
    }
}