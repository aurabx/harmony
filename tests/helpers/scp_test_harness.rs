use harmony::adapters::dimse::DimseAdapter;
use harmony::adapters::ProtocolAdapter;
use harmony::config::config::Config;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Test harness for DICOM SCP integration tests
/// 
/// This struct manages the lifecycle of a DIMSE adapter and its SCP listeners
/// for testing purposes.
pub struct ScpTestHarness {
    config: Arc<Config>,
    network_name: String,
    shutdown: CancellationToken,
    adapter_handle: Option<JoinHandle<()>>,
}

impl ScpTestHarness {
    /// Create a new test harness
    pub fn new(config: Config, network_name: impl Into<String>) -> Self {
        Self {
            config: Arc::new(config),
            network_name: network_name.into(),
            shutdown: CancellationToken::new(),
            adapter_handle: None,
        }
    }

    /// Start the DIMSE adapter and wait for SCP listeners to be ready
    pub async fn start(&mut self) -> anyhow::Result<()> {
        // Initialize globals (storage, config)
        harmony::globals::set_config(self.config.clone());
        
        let storage = harmony::storage::create_storage_backend(&self.config.storage)?;
        harmony::globals::set_storage(storage);

        // Create and start the DIMSE adapter
        let adapter = DimseAdapter::new(self.network_name.clone());
        let handle = adapter
            .start(self.config.clone(), self.shutdown.clone())
            .await?;

        self.adapter_handle = Some(handle);

        // Give the adapter a moment to start listeners
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        Ok(())
    }

    /// Shutdown the test harness gracefully
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        // Trigger shutdown
        self.shutdown.cancel();

        // Wait for adapter to complete
        if let Some(handle) = self.adapter_handle.take() {
            tokio::time::timeout(tokio::time::Duration::from_secs(5), handle)
                .await
                .map_err(|_| anyhow::anyhow!("Adapter shutdown timeout"))??;
        }

        Ok(())
    }

    /// Get the shutdown token (for manual control)
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }
}

impl Drop for ScpTestHarness {
    fn drop(&mut self) {
        // Ensure shutdown is triggered if not already
        self.shutdown.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmony::config::config::Config;
    use harmony::models::backends::backends::Backend;
    use harmony::models::endpoints::endpoint::Endpoint;
    use harmony::models::network::config::{HttpConfig, NetworkConfig};
    use harmony::models::pipelines::config::Pipeline;
    use serde_json::json;

    #[tokio::test]
    async fn test_harness_lifecycle() {
        let mut config = Config::default();

        // Create a test network
        let mut network = NetworkConfig::default();
        network.http = HttpConfig {
            bind_address: "127.0.0.1".to_string(),
            bind_port: 8090,
        };
        config.network.insert("test_network".to_string(), network);

        // Create a dicom_scp endpoint
        config.endpoints.insert(
            "test_scp".to_string(),
            Endpoint {
                service: "dicom_scp".to_string(),
                options: Some(
                    json!({
                        "local_aet": "TEST_SCP",
                        "port": 11120,
                        "enable_echo": true
                    })
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                ),
            },
        );

        // Create a pipeline
        config.pipelines.insert(
            "test_pipeline".to_string(),
            Pipeline {
                description: "Test pipeline".to_string(),
                networks: vec!["test_network".to_string()],
                endpoints: vec!["test_scp".to_string()],
                backends: vec![],
                ..Default::default()
            },
        );

        let mut harness = ScpTestHarness::new(config, "test_network");
        
        // Start should succeed
        harness.start().await.expect("harness should start");

        // Shutdown should succeed
        harness.shutdown().await.expect("harness should shutdown");
    }
}
