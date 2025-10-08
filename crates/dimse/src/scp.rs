//! Service Class Provider (SCP) implementation for inbound DIMSE operations

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, error, info, span, warn, Level};

use crate::config::DimseConfig;
use crate::router::{DimseRequest, DimseRequestPayload, DimseResponse, Router};
use crate::types::{DatasetStream, QueryLevel};
use crate::{DimseError, Result};

/// Trait for providing query capabilities to the SCP
#[async_trait]
pub trait QueryProvider: Send + Sync {
    /// Find datasets matching the given query
    async fn find(
        &self,
        query_level: QueryLevel,
        parameters: &std::collections::HashMap<String, String>,
        max_results: u32,
    ) -> Result<Vec<DatasetStream>>;

    /// Locate datasets for move operations
    async fn locate(
        &self,
        query_level: QueryLevel,
        parameters: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DatasetStream>>;

    /// Store a dataset (for C-STORE operations)
    async fn store(&self, dataset: DatasetStream) -> Result<()>;
}

/// DIMSE Service Class Provider
pub struct DimseScp {
    config: DimseConfig,
    #[allow(dead_code)]
    query_provider: Arc<dyn QueryProvider>, // TODO: Used for database queries
    router: Option<Arc<dyn Router>>,
    active_associations: Arc<RwLock<u32>>,
}

impl DimseScp {
    /// Create a new SCP with the given configuration and query provider
    pub fn new(config: DimseConfig, query_provider: Arc<dyn QueryProvider>) -> Self {
        Self {
            config,
            query_provider,
            router: None,
            active_associations: Arc::new(RwLock::new(0)),
        }
    }

    /// Set the router for handling requests
    pub fn with_router(mut self, router: Arc<dyn Router>) -> Self {
        self.router = Some(router);
        self
    }

    /// Start the SCP listener
    pub async fn run(self) -> Result<()> {
        let addr = SocketAddr::new(self.config.bind_addr, self.config.port);
        let listener = TcpListener::bind(addr).await?;

        info!(
            "Starting DIMSE SCP on {} (AET: {})",
            addr, self.config.local_aet
        );

        // Validate configuration
        self.config.validate()?;

        let scp = Arc::new(self);

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    debug!("Accepted connection from {}", peer_addr);

                    // Check association limit
                    {
                        let active = scp.active_associations.read().await;
                        if *active >= scp.config.max_associations {
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
                            error!("Error handling association from {}: {}", peer_addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting connection: {}", e);
                }
            }
        }
    }

    /// Handle a single association
    async fn handle_association(
        &self,
        stream: tokio::net::TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        // Increment active associations
        {
            let mut active = self.active_associations.write().await;
            *active += 1;
        }

        let result = self.handle_association_inner(stream, peer_addr).await;

        // Decrement active associations
        {
            let mut active = self.active_associations.write().await;
            *active -= 1;
        }

        result
    }

    /// Inner association handler
    async fn handle_association_inner(
        &self,
        _stream: tokio::net::TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        info!("Starting association with {}", peer_addr);

        // TODO: Implement actual DICOM UL association handling
        // This is a stub implementation that will be expanded with actual DICOM protocol handling

        // For now, just simulate handling requests via the router if available
        if let Some(router) = self.router.clone() {
            self.handle_router_requests(router).await?;
        }

        info!("Association with {} completed", peer_addr);
        Ok(())
    }

    /// Handle requests from the router (for testing and HTTP integration)
    async fn handle_router_requests(&self, _router: Arc<dyn Router>) -> Result<()> {
        // This is a placeholder - in a real implementation, we would need a different approach
        // since Router trait requires mutable access
        Ok(())
    }
    #[allow(dead_code)]
    async fn handle_dimse_request(
        &self,
        request: DimseRequest,
        router: &Arc<dyn Router>,
    ) -> Result<()> {
        let request_id = request.id;
        let _span =
            span!(Level::DEBUG, "dimse_request", id = %request_id, command = ?request.command)
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
                    .find(query.query_level, &query.parameters, query.max_results)
                    .await
                {
                    Ok(datasets) => {
                        debug!("Found {} matching datasets", datasets.len());

                        // Send each dataset as a pending response
                        for (i, dataset) in datasets.iter().enumerate() {
                            let is_final = i == datasets.len() - 1;
                            let response =
                                DimseResponse::find(request_id, Some(dataset.clone()), is_final);

                            if let Some(ref stream_tx) = request.stream_tx {
                                stream_tx.send(response).await.map_err(|_| {
                                    DimseError::router("Failed to send stream response")
                                })?;
                            }
                        }

                        // Send final empty response if no datasets found
                        if datasets.is_empty() {
                            let response = DimseResponse::find(request_id, None, true);
                            self.send_response(request, response, router).await?;
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
                    .locate(query.query_level, &query.parameters)
                    .await
                {
                    Ok(datasets) => {
                        let total = datasets.len() as u32;
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

            DimseRequestPayload::Store(ref dataset) => {
                debug!("Processing C-STORE request");

                match self.query_provider.store(dataset.clone()).await {
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

/// Default query provider implementation (for testing)
pub struct DefaultQueryProvider {
    storage_dir: std::path::PathBuf,
}

impl DefaultQueryProvider {
    pub fn new(storage_dir: std::path::PathBuf) -> Self {
        Self { storage_dir }
    }
}

#[async_trait]
impl QueryProvider for DefaultQueryProvider {
    async fn find(
        &self,
        _query_level: QueryLevel,
        _parameters: &std::collections::HashMap<String, String>,
        _max_results: u32,
    ) -> Result<Vec<DatasetStream>> {
        // TODO: Implement actual query logic
        warn!("DefaultQueryProvider::find not yet implemented");
        Ok(vec![])
    }

    async fn locate(
        &self,
        _query_level: QueryLevel,
        _parameters: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DatasetStream>> {
        // TODO: Implement actual locate logic
        warn!("DefaultQueryProvider::locate not yet implemented");
        Ok(vec![])
    }

    async fn store(&self, dataset: DatasetStream) -> Result<()> {
        // Store the dataset to the storage directory
        let temp_file = dataset.to_temp_file(&self.storage_dir).await?;
        info!("Stored dataset to {}", temp_file.display());
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
