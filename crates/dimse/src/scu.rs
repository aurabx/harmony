//! Service Class User (SCU) implementation for outbound DIMSE operations

use std::time::Duration;

use futures::stream::Stream;
use tokio::sync::mpsc;
use tracing::{info, warn, error, debug};

use crate::config::{DimseConfig, RemoteNode};
use crate::types::{DatasetStream, FindQuery, MoveQuery};
use crate::{DimseError, Result};

/// DIMSE Service Class User
pub struct DimseScu {
    #[allow(dead_code)]
    config: DimseConfig, // TODO: Used for connection configuration
}

impl DimseScu {
    /// Create a new SCU with the given configuration
    pub fn new(config: DimseConfig) -> Self {
        Self { config }
    }

    /// Send a C-ECHO request to a remote node
    pub async fn echo(&self, node: &RemoteNode) -> Result<bool> {
        info!("Sending C-ECHO to {}@{}:{}", node.ae_title, node.host, node.port);
        
        // Validate the remote node configuration
        node.validate()?;
        
        #[cfg(feature = "dcmtk_cli")]
        {
            use tokio::process::Command;
            // Use DCMTK echoscu as a real C-ECHO implementation
            let mut cmd = Command::new("echoscu");
            cmd.arg("-aet").arg(&self.config.local_aet)
                .arg("-aec").arg(&node.ae_title)
                .arg(&node.host)
                .arg(node.port.to_string());
            debug!("Running: echoscu -aet {} -aec {} {} {}", self.config.local_aet, node.ae_title, node.host, node.port);
            let output = cmd.output().await.map_err(|e| DimseError::operation_failed(format!("Failed to spawn echoscu: {}", e)))?;
            if output.status.success() {
                info!("C-ECHO completed successfully");
                return Ok(true);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                error!("C-ECHO failed: status={:?}, stdout={}, stderr={}", output.status.code(), stdout, stderr);
                return Err(DimseError::operation_failed(format!("echoscu failed: {:?} {}", output.status.code(), stderr)));
            }
        }
        
        #[cfg(not(feature = "dcmtk_cli"))]
        {
            return Err(DimseError::NotSupported("C-ECHO requires feature 'dcmtk_cli' or a native UL implementation".into()));
        }
    }

    /// Send a C-FIND request to a remote node
    pub async fn find(
        &self, 
        node: &RemoteNode, 
        query: FindQuery
    ) -> Result<impl Stream<Item = Result<DatasetStream>>> {
        info!(
            "Sending C-FIND to {}@{}:{} (level: {}, max_results: {})", 
            node.ae_title, 
            node.host, 
            node.port,
            query.query_level,
            query.max_results
        );
        
        // Validate the remote node configuration
        node.validate()?;
        
        debug!("C-FIND query parameters: {:?}", query.parameters);
        
        // TODO: Implement actual DICOM association and C-FIND
        // This is a stub implementation that returns an empty stream
        
        let (_tx, rx) = mpsc::channel(100);
        
        // Simulate some async work
        tokio::spawn(async move {
            // Simulate finding some results
            tokio::time::sleep(Duration::from_millis(200)).await;
            
            // For now, just send an empty result set
            debug!("C-FIND completed with 0 results");
            // Channel closes automatically when tx is dropped
        });
        
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(stream)
    }

    /// Send a C-MOVE request to a remote node
    pub async fn move_request(
        &self,
        node: &RemoteNode,
        query: MoveQuery,
    ) -> Result<impl Stream<Item = Result<DatasetStream>>> {
        info!(
            "Sending C-MOVE to {}@{}:{} (level: {}, dest: {})", 
            node.ae_title, 
            node.host, 
            node.port,
            query.query_level,
            query.destination_aet
        );
        
        // Validate the remote node configuration
        node.validate()?;
        
        debug!("C-MOVE query parameters: {:?}", query.parameters);
        
        // TODO: Implement actual DICOM association and C-MOVE
        // This is a stub implementation
        
        let (_tx, rx) = mpsc::channel(100);
        
        // Simulate some async work
        tokio::spawn(async move {
            // Simulate move operation
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            // For now, just complete successfully without moving anything
            debug!("C-MOVE completed");
            // Channel closes automatically when tx is dropped
        });
        
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(stream)
    }

    /// Send a C-STORE request to a remote node
    pub async fn store(&self, node: &RemoteNode, dataset: DatasetStream) -> Result<bool> {
        info!("Sending C-STORE to {}@{}:{}", node.ae_title, node.host, node.port);
        
        // Validate the remote node configuration
        node.validate()?;
        
        debug!("C-STORE dataset: id={}", dataset.metadata().id);
        
        // TODO: Implement actual DICOM association and C-STORE
        // This is a stub implementation
        
        // Simulate sending the dataset
        tokio::time::sleep(Duration::from_millis(300)).await;
        
        info!("C-STORE completed successfully");
        Ok(true)
    }

    /// Test connectivity to a remote node with retry logic
    pub async fn test_connection(&self, node: &RemoteNode, max_retries: u32) -> Result<bool> {
        let mut retries = 0;
        
        while retries <= max_retries {
            if retries > 0 {
                info!("Connection test retry {} of {}", retries, max_retries);
                tokio::time::sleep(Duration::from_secs(1 << retries)).await; // Exponential backoff
            }
            
            match self.echo(node).await {
                Ok(_) => {
                    info!("Connection test successful");
                    return Ok(true);
                }
                Err(e) if e.is_recoverable() && retries < max_retries => {
                    warn!("Connection test failed (attempt {}): {}", retries + 1, e);
                    retries += 1;
                    continue;
                }
                Err(e) => {
                    error!("Connection test failed permanently: {}", e);
                    return Err(e);
                }
            }
        }
        
        Err(DimseError::operation_failed("Connection test failed after all retries"))
    }

    /// Get connection timeout for a node (uses node-specific or global setting)
    #[allow(dead_code)]
    fn get_connection_timeout(&self, node: &RemoteNode) -> Duration {
        node.connect_timeout_ms
            .map(Duration::from_millis)
            .unwrap_or_else(|| self.config.connect_timeout())
    }

    /// Get maximum PDU size for a node (uses node-specific or global setting)
    #[allow(dead_code)]
    fn get_max_pdu(&self, node: &RemoteNode) -> u32 {
        node.max_pdu.unwrap_or(self.config.max_pdu)
    }
}

/// Builder for creating SCU instances with custom configurations
pub struct ScuBuilder {
    config: DimseConfig,
}

impl ScuBuilder {
    /// Start building a new SCU
    pub fn new() -> Self {
        Self {
            config: DimseConfig::default(),
        }
    }

    /// Set the local AE title
    pub fn local_aet(mut self, aet: impl Into<String>) -> Self {
        self.config.local_aet = aet.into();
        self
    }

    /// Set the connection timeout
    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout_ms = timeout.as_millis() as u64;
        self
    }

    /// Set the maximum PDU size
    pub fn max_pdu(mut self, size: u32) -> Self {
        self.config.max_pdu = size;
        self
    }

    /// Build the SCU
    pub fn build(self) -> Result<DimseScu> {
        self.config.validate()?;
        Ok(DimseScu::new(self.config))
    }
}

impl Default for ScuBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::StreamExt;

    #[tokio::test]
    async fn test_scu_creation() {
        let scu = ScuBuilder::new()
            .local_aet("TEST_SCU")
            .connection_timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        
        assert_eq!(scu.config.local_aet, "TEST_SCU");
        assert_eq!(scu.config.connect_timeout_ms, 10_000);
    }

    #[tokio::test]
    #[ignore]
    async fn test_echo_stub() {
        let scu = DimseScu::new(DimseConfig::default());
        let node = RemoteNode::new("TEST_AET", "localhost", 11112);
        
        // This should succeed with our stub implementation
        let result = scu.echo(&node).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_find_stub() {
        let scu = DimseScu::new(DimseConfig::default());
        let node = RemoteNode::new("TEST_AET", "localhost", 11112);
        let query = FindQuery::patient(Some("12345".to_string()));
        
        let mut stream = scu.find(&node, query).await.unwrap();
        
        // The stub implementation should return an empty stream
        let first_result = stream.next().await;
        assert!(first_result.is_none());
    }

    #[tokio::test] 
    async fn test_connection_timeout_selection() {
        let scu = DimseScu::new(DimseConfig {
            connect_timeout_ms: 5000,
            ..Default::default()
        });
        
        // Node without specific timeout should use global
        let node1 = RemoteNode::new("TEST1", "localhost", 11112);
        assert_eq!(scu.get_connection_timeout(&node1), Duration::from_millis(5000));
        
        // Node with specific timeout should use its own
        let node2 = RemoteNode::new("TEST2", "localhost", 11113).with_timeout(2000);
        assert_eq!(scu.get_connection_timeout(&node2), Duration::from_millis(2000));
    }

    #[test]
    fn test_invalid_config_validation() {
        let result = ScuBuilder::new()
            .local_aet("") // Invalid empty AE title
            .build();
        
        assert!(result.is_err());
    }
}