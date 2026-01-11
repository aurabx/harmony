use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::models::envelope::envelope::ResponseEnvelope;
use crate::models::protocol::{Protocol, ProtocolCtx};
use crate::utils::Error;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use http::StatusCode;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub mod common;
pub mod content_type;
pub mod router;

/// HTTP Protocol Adapter
///
/// Wraps Axum HTTP server and provides protocol-specific I/O handling
/// while using the common PipelineExecutor for business logic.
/// Supports both plain HTTP and HTTPS via optional TLS configuration.
pub struct HttpAdapter {
    pub network_name: String,
    pub bind_addr: SocketAddr,
    pub tls_config: Option<TlsConfig>,
    pub force_https: bool,
}

/// TLS configuration for HTTPS
#[derive(Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

/// Middleware to redirect HTTP requests to HTTPS
async fn https_redirect_middleware(
    req: Request,
    _next: axum::middleware::Next,
) -> Response {
    // Get the host header
    let host = req
        .headers()
        .get(http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    
    // Build HTTPS URL
    let uri = req.uri();
    let path_and_query = uri.path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    
    let https_url = format!("https://{}{}", host, path_and_query);
    
    // Return 301 redirect
    Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header(http::header::LOCATION, https_url)
        .body(Body::empty())
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap()
        })
}

impl HttpAdapter {
    pub fn new(network_name: String, bind_addr: SocketAddr, tls_config: Option<TlsConfig>, force_https: bool) -> Self {
        Self {
            network_name,
            bind_addr,
            tls_config,
            force_https,
        }
    }

    /// Convert Axum HTTP Request to ProtocolCtx
    ///
    /// This method extracts the request body and delegates to the shared
    /// `common::build_protocol_ctx` helper for building the context.
    pub async fn http_request_to_protocol_ctx(
        req: &mut Request,
        options: &HashMap<String, serde_json::Value>,
    ) -> Result<ProtocolCtx, Error> {
        // Read body bytes from the Axum request
        let body_bytes =
            axum::body::to_bytes(std::mem::replace(req.body_mut(), Body::empty()), usize::MAX)
                .await
                .map_err(|_| Error::from("Failed to read request body"))?
                .to_vec();

        // Get path prefix from options
        let path_prefix = common::get_path_prefix(options);

        // Use shared helper to build the ProtocolCtx
        Ok(common::build_protocol_ctx(
            req.method(),
            req.uri(),
            req.headers(),
            body_bytes,
            path_prefix,
            "http",
        ))
    }

    /// Build rustls ServerConfig from TLS settings
    fn build_tls_config(tls: &TlsConfig) -> anyhow::Result<rustls::ServerConfig> {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
        use std::fs::File;
        use std::io::BufReader;

        // Load certificate chain
        let mut cert_file = BufReader::new(File::open(&tls.cert_path)?);
        let cert_chain: Vec<CertificateDer<'static>> = certs(&mut cert_file)
            .collect::<Result<_, _>>()?;

        if cert_chain.is_empty() {
            anyhow::bail!("no certificates found in {}", tls.cert_path);
        }

        // Load private key (try PKCS#8 first, then RSA)
        let mut key_file = BufReader::new(File::open(&tls.key_path)?);
        let mut keys: Vec<PrivateKeyDer<'static>> = pkcs8_private_keys(&mut key_file)
            .map(|r| r.map(|k| PrivateKeyDer::Pkcs8(k)))
            .collect::<Result<_, _>>()?;

        if keys.is_empty() {
            // Rewind and try RSA
            let mut key_file = BufReader::new(File::open(&tls.key_path)?);
            keys = rsa_private_keys(&mut key_file)
                .map(|r| r.map(|k| PrivateKeyDer::Pkcs1(k)))
                .collect::<Result<_, _>>()?;
        }

        let key = keys
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no private keys found in {}", tls.key_path))?;

        // Build rustls config with HTTP/1.1 and HTTP/2 ALPN
        let mut tls_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)?;

        tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        Ok(tls_config)
    }

    /// Convert ResponseEnvelope to Axum HTTP Response
    pub fn response_envelope_to_http(
        envelope: ResponseEnvelope<Vec<u8>>,
    ) -> Result<Response<Body>, Error> {
        let mut builder = Response::builder().status(envelope.response_details.status);

        // Add headers
        for (key, value) in &envelope.response_details.headers {
            builder = builder.header(key, value);
        }

        // Build response with body
        builder
            .body(Body::from(envelope.original_data))
            .map_err(|e| Error::from(format!("Failed to build HTTP response: {}", e)))
    }
}

#[async_trait]
impl ProtocolAdapter for HttpAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::Http
    }

    fn from_network(
        network_name: String,
        network_config: &crate::models::network::config::NetworkConfig,
    ) -> Box<dyn ProtocolAdapter> {
        let tcp = network_config.tcp_config.as_ref().unwrap_or_else(|| {
            panic!(
                "HttpAdapter requested for network '{}' but no TCP HTTP config (network.<name>.http / tcp_config) was provided",
                network_name
            )
        });

        let bind_addr = format!("{}:{}", tcp.bind_address, tcp.bind_port)
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| {
                panic!(
                    "Invalid TCP bind address or port for network '{}': {}:{}",
                    network_name, tcp.bind_address, tcp.bind_port
                )
            });

        // Extract TLS configuration if both cert and key paths are present
        let tls_config = match (&tcp.cert_path, &tcp.key_path) {
            (Some(cert), Some(key)) => Some(TlsConfig {
                cert_path: cert.clone(),
                key_path: key.clone(),
            }),
            _ => None,
        };

        let force_https = tcp.force_https;

        Box::new(HttpAdapter::new(network_name, bind_addr, tls_config, force_https))
    }

    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let bind_addr = self.bind_addr;
        let network_name = self.network_name.clone();
        let tls_config = self.tls_config.clone();
        let force_https = self.force_https;

        // Build the router using the router module
        let mut app = router::build_network_router(config.clone(), &network_name).await;
        
        // Wrap app with HTTPS redirect layer if force_https is enabled and no TLS
        if force_https && tls_config.is_none() {
            app = app.layer(axum::middleware::from_fn(https_redirect_middleware));
        }

        Ok(tokio::spawn(async move {
            // Check if TLS is configured
            if let Some(tls) = tls_config {
                // HTTPS mode with TLS
                let rustls_config = match Self::build_tls_config(&tls) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        tracing::error!(
                            "Failed to build TLS config for network '{}': {}",
                            network_name,
                            e
                        );
                        return;
                    }
                };

                tracing::info!(
                    "🚀 HTTPS adapter started for network '{}' on {}",
                    network_name,
                    bind_addr
                );

                // Create axum-server RustlsConfig from rustls::ServerConfig
                let axum_tls_config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(rustls_config));

                let handle = axum_server::Handle::new();
                let shutdown_handle = handle.clone();

                // Spawn shutdown watcher
                tokio::spawn(async move {
                    shutdown.cancelled().await;
                    shutdown_handle.shutdown();
                });

                if let Err(e) = axum_server::bind_rustls(bind_addr, axum_tls_config)
                    .handle(handle)
                    .serve(app.into_make_service())
                    .await
                {
                    tracing::error!(
                        "HTTPS adapter for network '{}' encountered error: {}",
                        network_name,
                        e
                    );
                }

                tracing::info!("HTTPS adapter for network '{}' shut down", network_name);
            } else {
                // Plain HTTP mode
                let listener = match TcpListener::bind(bind_addr).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!(
                            "Failed to bind HTTP adapter for network '{}' to {}: {}",
                            network_name,
                            bind_addr,
                            e
                        );
                        return;
                    }
                };

                tracing::info!(
                    "🚀 HTTP adapter started for network '{}' on {}",
                    network_name,
                    bind_addr
                );

                let graceful_shutdown = async move {
                    shutdown.cancelled().await;
                };

                if let Err(e) = axum::serve(listener, app)
                    .with_graceful_shutdown(graceful_shutdown)
                    .await
                {
                    tracing::error!(
                        "HTTP adapter for network '{}' encountered error: {}",
                        network_name,
                        e
                    );
                }

                tracing::info!("HTTP adapter for network '{}' shut down", network_name);
            }
        }))
    }

    fn summary(&self) -> String {
        let protocol = if self.tls_config.is_some() { "HTTPS" } else { "HTTP" };
        format!(
            "HttpAdapter(network={}, bind={}, protocol={})",
            self.network_name, self.bind_addr, protocol
        )
    }
}
