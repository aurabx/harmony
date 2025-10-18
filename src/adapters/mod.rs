pub mod dimse;
pub mod http;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config::Config;
    use std::sync::Arc;
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    // Test adapter implementation for testing the trait
    struct TestAdapter {
        protocol: Protocol,
    }

    #[async_trait]
    impl ProtocolAdapter for TestAdapter {
        fn protocol(&self) -> Protocol {
            self.protocol
        }

        async fn start(
            &self,
            _config: Arc<Config>,
            shutdown: CancellationToken,
        ) -> anyhow::Result<JoinHandle<()>> {
            let protocol = self.protocol;
            Ok(tokio::spawn(async move {
                // Wait for shutdown
                shutdown.cancelled().await;
                tracing::info!("TestAdapter for {:?} shut down", protocol);
            }))
        }

        fn summary(&self) -> String {
            format!("TestAdapter for {:?}", self.protocol)
        }
    }

    #[test]
    fn test_protocol_adapter_trait_object_safety() {
        // Ensure ProtocolAdapter is object-safe (can be used as dyn)
        let adapter: Box<dyn ProtocolAdapter> = Box::new(TestAdapter {
            protocol: Protocol::Http,
        });
        
        assert_eq!(adapter.protocol(), Protocol::Http);
        assert_eq!(adapter.summary(), "TestAdapter for Http");
    }

    #[tokio::test]
    async fn test_adapter_lifecycle() {
        let adapter = TestAdapter {
            protocol: Protocol::Dimse,
        };
        
        let config = Arc::new(Config::default());
        let shutdown = CancellationToken::new();
        
        // Start adapter
        let handle = adapter.start(config, shutdown.clone()).await.unwrap();
        
        // Trigger shutdown
        shutdown.cancel();
        
        // Wait for adapter to shut down
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Adapter should shut down gracefully");
    }

    #[tokio::test]
    async fn test_multiple_adapters_with_same_shutdown() {
        let http_adapter = TestAdapter {
            protocol: Protocol::Http,
        };
        let dimse_adapter = TestAdapter {
            protocol: Protocol::Dimse,
        };
        
        let config = Arc::new(Config::default());
        let shutdown = CancellationToken::new();
        
        // Start both adapters with same shutdown token
        let http_handle = http_adapter.start(config.clone(), shutdown.clone()).await.unwrap();
        let dimse_handle = dimse_adapter.start(config, shutdown.clone()).await.unwrap();
        
        // Trigger shutdown for both
        shutdown.cancel();
        
        // Both should shut down
        let http_result = tokio::time::timeout(std::time::Duration::from_secs(1), http_handle).await;
        let dimse_result = tokio::time::timeout(std::time::Duration::from_secs(1), dimse_handle).await;
        
        assert!(http_result.is_ok(), "HTTP adapter should shut down");
        assert!(dimse_result.is_ok(), "DIMSE adapter should shut down");
    }

    #[test]
    fn test_protocol_adapter_is_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        
        assert_send::<TestAdapter>();
        assert_sync::<TestAdapter>();
    }
}
