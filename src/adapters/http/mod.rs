use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::models::envelope::envelope::ResponseEnvelope;
use crate::models::protocol::{Protocol, ProtocolCtx};
use crate::utils::Error;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
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
pub struct HttpAdapter {
    pub network_name: String,
    pub bind_addr: SocketAddr,
}

impl HttpAdapter {
    pub fn new(network_name: String, bind_addr: SocketAddr) -> Self {
        Self {
            network_name,
            bind_addr,
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
        Box::new(HttpAdapter::new(network_name, bind_addr))
    }

    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let bind_addr = self.bind_addr;
        let network_name = self.network_name.clone();

        // Build the router using the router module
        let app = router::build_network_router(config.clone(), &network_name).await;

        Ok(tokio::spawn(async move {
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

            // Create a future for graceful shutdown
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
        }))
    }

    fn summary(&self) -> String {
        format!(
            "HttpAdapter(network={}, bind={})",
            self.network_name, self.bind_addr
        )
    }
}
