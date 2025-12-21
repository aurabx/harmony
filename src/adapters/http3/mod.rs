use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::models::protocol::Protocol;
use async_trait::async_trait;
use quinn::{Endpoint, ServerConfig};
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
        use std::io::{BufReader, Read};

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
            .collect::<Result<_, _>>()?;

        if keys.is_empty() {
            // Rewind and try RSA
            let mut key_file = BufReader::new(File::open(&self.key_path)?);
            keys = rsa_private_keys(&mut key_file)
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
                        Some(connecting) => {
                            let config = config.clone();
                            let network_name = self.network_name.clone();
                            tokio::spawn(async move {
                                if let Err(e) = super::http3::handle_connection(connecting, config, network_name).await {
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