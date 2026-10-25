//! Service Class Provider (SCP) implementation for inbound DIMSE operations

pub mod association;
pub mod commands;
pub mod message_assembler;
pub mod pdu_handler;
pub mod query_provider;
pub mod response_builder;

pub use query_provider::DefaultQueryProvider;

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};

use crate::config::DimseConfig;
use crate::router::{DimseRequest, DimseRequestPayload, DimseResponse, Router};
use crate::types::{BoxDatasetStream, DatasetStream, QueryLevel, RetrieveStream};
use crate::{DimseError, Result};

/// Trait for providing query capabilities to the SCP.
///
/// Query and retrieve results are streams, so the SCP can answer the client
/// with each dataset as it arrives instead of waiting for the whole result.
///
/// Every method receives `peer_addr`, the TCP address of the association's
/// requestor. Providers use it for access control, rate limiting and audit.
#[async_trait]
pub trait QueryProvider: Send + Sync {
    /// Find datasets matching the given query
    async fn find(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &std::collections::HashMap<String, String>,
        max_results: u32,
    ) -> Result<BoxDatasetStream>;

    /// Locate datasets for move operations
    async fn locate(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &std::collections::HashMap<String, String>,
    ) -> Result<BoxDatasetStream>;

    /// Retrieve datasets for get and move operations (C-GET, C-MOVE)
    async fn get(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &std::collections::HashMap<String, String>,
    ) -> Result<RetrieveStream>;

    /// Store a dataset (for C-STORE operations)
    async fn store(&self, peer_addr: SocketAddr, dataset: DatasetStream) -> Result<()>;

    /// Decide whether `peer_addr` may open an association.
    ///
    /// The SCP calls this once per TCP connection, before it answers the
    /// A-ASSOCIATE-RQ. A rejected peer gets an A-ASSOCIATE-RJ and never
    /// reaches any command, C-ECHO included. An `Err` is treated as a reject,
    /// so a provider that cannot decide fails closed.
    ///
    /// The default admits every peer, which is the behaviour of an SCP with no
    /// access control. Providers that enforce network policy override it.
    async fn admit_peer(&self, _peer_addr: SocketAddr) -> Result<PeerAdmission> {
        Ok(PeerAdmission::Admit)
    }
}

/// Outcome of [`QueryProvider::admit_peer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerAdmission {
    /// The peer may negotiate an association.
    Admit,
    /// The peer is refused. `reason` is logged by the SCP; it is not sent to
    /// the peer, because A-ASSOCIATE-RJ carries only a fixed reason code.
    Reject { reason: String },
}

/// DIMSE Service Class Provider
pub struct DimseScp {
    pub(crate) config: DimseConfig,
    pub(crate) query_provider: Arc<dyn QueryProvider>,
    router: Option<Arc<dyn Router>>,
    pub(crate) active_associations: Arc<AtomicU32>,
}

impl DimseScp {
    /// Create a new SCP with the given configuration and query provider
    pub fn new(config: DimseConfig, query_provider: Arc<dyn QueryProvider>) -> Self {
        Self {
            config,
            query_provider,
            router: None,
            active_associations: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Set the router for handling requests
    pub fn with_router(mut self, router: Arc<dyn Router>) -> Self {
        self.router = Some(router);
        self
    }

    /// Start the SCP listener
    pub async fn run(self, shutdown: tokio_util::sync::CancellationToken) -> Result<()> {
        let addr = SocketAddr::new(self.config.bind_addr, self.config.port);
        // Use socket2 for reliable bind with SO_REUSEADDR (helps with ephemeral port races in tests)
        let socket = socket2::Socket::new(
            match addr {
                SocketAddr::V4(_) => socket2::Domain::IPV4,
                SocketAddr::V6(_) => socket2::Domain::IPV6,
            },
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;
        socket.set_reuse_address(true)?;
        socket.bind(&addr.into())?;
        socket.listen(128)?;
        let std_listener: std::net::TcpListener = socket.into();
        std_listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(std_listener)?;

        info!(
            "Starting DIMSE SCP on {} (AET: {})",
            addr, self.config.local_aet
        );

        // Validate configuration
        self.config.validate()?;

        let scp = Arc::new(self);

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            debug!("Accepted connection from {}", peer_addr);

                            // Check association limit
                            {
                                let active = scp.active_associations.load(Ordering::Relaxed);
                                if active >= scp.config.max_associations {
                                    warn!(
                                        "Maximum associations reached, rejecting connection from {}",
                                        peer_addr
                                    );
                                    drop(stream);
                                    continue;
                                }
                            }

                            let scp_clone = Arc::clone(&scp);
                            tokio::spawn(async move {
                                if let Err(e) = scp_clone.handle_association(stream, peer_addr).await {
                                    // "Connection closed by peer" during handshake is usually a health check
                                    let err_msg = e.to_string();
                                    if err_msg.contains("Connection closed by peer") {
                                        debug!("Connection from {} closed during handshake (likely health check)", peer_addr);
                                    } else {
                                        error!("Error handling association from {}: {}", peer_addr, e);
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            error!("Error accepting connection: {}", e);
                        }
                    }
                }
                _ = shutdown.cancelled() => {
                    info!("Shutdown signal received, stopping SCP listener");
                    break;
                }
            }
        }

        info!("DIMSE SCP listener stopped");
        Ok(())
    }

    /// Handle a single association
    pub(crate) async fn handle_association(
        &self,
        stream: tokio::net::TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        association::handle_association(self, stream, peer_addr).await
    }

    #[allow(dead_code)]
    async fn handle_dimse_request(
        &self,
        peer_addr: SocketAddr,
        request: DimseRequest,
        router: &Arc<dyn Router>,
    ) -> Result<()> {
        let request_id = request.id;
        let _span = tracing::span!(
            tracing::Level::DEBUG,
            "dimse_request",
            id = %request_id,
            command = ?request.command
        )
        .entered();

        match request.payload {
            DimseRequestPayload::Echo => {
                debug!("Processing C-ECHO request");
                let response = if self.config.enable_echo {
                    DimseResponse::echo(request_id, true)
                } else {
                    DimseResponse::error(request_id, "C-ECHO not supported".to_string())
                };

                self.send_response(request, response, router).await?;
            }

            DimseRequestPayload::Find(ref query) => {
                debug!(
                    "Processing C-FIND request: level={}, params={:?}",
                    query.query_level, query.parameters
                );

                if !self.config.enable_find {
                    let response =
                        DimseResponse::error(request_id, "C-FIND not supported".to_string());
                    self.send_response(request, response, router).await?;
                    return Ok(());
                }

                match self
                    .query_provider
                    .find(
                        peer_addr,
                        query.query_level,
                        &query.parameters,
                        query.max_results,
                    )
                    .await
                {
                    Ok(mut datasets) => {
                        // Each dataset goes out as a pending response. The last one is
                        // only known once the stream ends, so one dataset is held back.
                        let mut held: Option<DatasetStream> = None;
                        let mut count = 0usize;
                        while let Some(item) = datasets.next().await {
                            let dataset = match item {
                                Ok(dataset) => dataset,
                                Err(e) => {
                                    warn!("C-FIND result skipped: {}", e);
                                    continue;
                                }
                            };
                            if let Some(previous) = held.replace(dataset) {
                                count += 1;
                                let response =
                                    DimseResponse::find(request_id, Some(previous), false);
                                if let Some(ref stream_tx) = request.stream_tx {
                                    stream_tx.send(response).await.map_err(|_| {
                                        DimseError::router("Failed to send stream response")
                                    })?;
                                }
                            }
                        }
                        debug!(
                            "Found {} matching datasets",
                            count + usize::from(held.is_some())
                        );

                        match held {
                            Some(last) => {
                                let response = DimseResponse::find(request_id, Some(last), true);
                                if let Some(ref stream_tx) = request.stream_tx {
                                    stream_tx.send(response).await.map_err(|_| {
                                        DimseError::router("Failed to send stream response")
                                    })?;
                                }
                            }
                            None => {
                                let response = DimseResponse::find(request_id, None, true);
                                self.send_response(request, response, router).await?;
                            }
                        }
                    }
                    Err(e) => {
                        let response = DimseResponse::error(request_id, e.to_string());
                        self.send_response(request, response, router).await?;
                    }
                }
            }

            DimseRequestPayload::Move(ref query) => {
                debug!(
                    "Processing C-MOVE request: level={}, dest={}",
                    query.query_level, query.destination_aet
                );

                if !self.config.enable_move {
                    let response =
                        DimseResponse::error(request_id, "C-MOVE not supported".to_string());
                    self.send_response(request, response, router).await?;
                    return Ok(());
                }

                // TODO: Implement actual C-MOVE logic
                // For now, just locate the datasets and report status
                match self
                    .query_provider
                    .locate(peer_addr, query.query_level, &query.parameters)
                    .await
                {
                    Ok(datasets) => {
                        let total = datasets.count().await as u32;
                        debug!("Located {} datasets for move", total);

                        // Send final status response
                        let response = DimseResponse::move_response(
                            request_id, None, 0,     // remaining
                            total, // completed
                            0,     // failed
                            0,     // warning
                            true,  // is_final
                        );
                        self.send_response(request, response, router).await?;
                    }
                    Err(e) => {
                        let response = DimseResponse::error(request_id, e.to_string());
                        self.send_response(request, response, router).await?;
                    }
                }
            }

            DimseRequestPayload::Get(ref query) => {
                debug!("Processing C-GET request: level={}", query.query_level);

                if !self.config.enable_get {
                    let response =
                        DimseResponse::error(request_id, "C-GET not supported".to_string());
                    self.send_response(request, response, router).await?;
                    return Ok(());
                }

                match self
                    .query_provider
                    .get(peer_addr, query.query_level, &query.parameters)
                    .await
                {
                    Ok(retrieve) => {
                        let total = retrieve.expected_total.map(u32::from);
                        let mut datasets = retrieve.datasets;
                        debug!("Retrieving datasets for get (expected total: {:?})", total);

                        // One dataset is held back so the last response can be marked final.
                        let mut held: Option<DatasetStream> = None;
                        let mut completed = 0u32;
                        while let Some(item) = datasets.next().await {
                            let dataset = match item {
                                Ok(dataset) => dataset,
                                Err(e) => {
                                    warn!("C-GET dataset skipped: {}", e);
                                    continue;
                                }
                            };
                            if let Some(previous) = held.replace(dataset) {
                                completed += 1;
                                let remaining = total.map_or(0, |t| t.saturating_sub(completed));
                                let response = DimseResponse::get_response(
                                    request_id,
                                    Some(previous),
                                    remaining,
                                    completed,
                                    0, // failed
                                    0, // warning
                                    false,
                                );
                                if let Some(ref stream_tx) = request.stream_tx {
                                    stream_tx.send(response).await.map_err(|_| {
                                        DimseError::router("Failed to send stream response")
                                    })?;
                                }
                            }
                        }

                        match held {
                            Some(last) => {
                                completed += 1;
                                let response = DimseResponse::get_response(
                                    request_id,
                                    Some(last),
                                    0,
                                    completed,
                                    0, // failed
                                    0, // warning
                                    true,
                                );
                                if let Some(ref stream_tx) = request.stream_tx {
                                    stream_tx.send(response).await.map_err(|_| {
                                        DimseError::router("Failed to send stream response")
                                    })?;
                                }
                            }
                            None => {
                                let response =
                                    DimseResponse::get_response(request_id, None, 0, 0, 0, 0, true);
                                self.send_response(request, response, router).await?;
                            }
                        }
                    }
                    Err(e) => {
                        let response = DimseResponse::error(request_id, e.to_string());
                        self.send_response(request, response, router).await?;
                    }
                }
            }

            DimseRequestPayload::Store(ref dataset) => {
                debug!("Processing C-STORE request");

                match self.query_provider.store(peer_addr, dataset.clone()).await {
                    Ok(()) => {
                        let response = DimseResponse::store(request_id, true);
                        self.send_response(request, response, router).await?;
                    }
                    Err(e) => {
                        let response = DimseResponse::error(request_id, e.to_string());
                        self.send_response(request, response, router).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Send a response back through the appropriate channel
    #[allow(dead_code)]
    async fn send_response(
        &self,
        request: DimseRequest,
        response: DimseResponse,
        router: &Arc<dyn Router>,
    ) -> Result<()> {
        if let Some(response_tx) = request.response_tx {
            response_tx
                .send(response)
                .map_err(|_| DimseError::router("Failed to send response"))?;
        } else {
            router.send_response(response).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn test_scp_creation() {
        let config = DimseConfig {
            local_aet: "TEST_SCP".to_string(),
            bind_addr: std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            port: 0, // Use any available port
            ..Default::default()
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let query_provider = Arc::new(DefaultQueryProvider::new(temp_dir.path().to_path_buf()));

        let scp = DimseScp::new(config, query_provider);
        assert_eq!(scp.config.local_aet, "TEST_SCP");
    }

    #[test]
    fn test_default_query_provider() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _provider = DefaultQueryProvider::new(temp_dir.path().to_path_buf());
        // Basic creation test
    }
}
