use crate::config::config::Config;
use crate::models::protocol::Protocol;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Protocol adapter trait
/// 
/// Each protocol (HTTP, DIMSE, HL7, etc.) implements this trait to provide
/// protocol-specific I/O handling while using the common PipelineExecutor
/// for business logic.
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    /// Returns the protocol this adapter handles
    fn protocol(&self) -> Protocol;

    /// Start the adapter (listener, server, etc.)
    /// 
    /// # Arguments
    /// * `config` - Application configuration
    /// * `shutdown` - Cancellation token for graceful shutdown
    /// 
    /// # Returns
    /// JoinHandle for the adapter task
    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>>;

    /// Returns a human-readable summary of the adapter configuration
    /// Used for logging and debugging
    fn summary(&self) -> String;
}
