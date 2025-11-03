use super::{dimse::DimseAdapter, http::HttpAdapter, ProtocolAdapter};
use crate::config::config::Config;
use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Handle for a running adapter
pub struct AdapterHandle {
    pub network_name: String,
    pub adapter_summary: String,
    pub task_handle: JoinHandle<()>,
    pub shutdown_token: CancellationToken,
}

/// Registry for managing protocol adapters
pub struct AdapterRegistry {
    adapters: Arc<RwLock<HashMap<String, Vec<AdapterHandle>>>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start all adapters for a network
    pub async fn start_network(
        &self,
        network_name: String,
        config: Arc<Config>,
    ) -> Result<()> {
        let network = config
            .network
            .get(&network_name)
            .ok_or_else(|| anyhow::anyhow!("Network '{}' not found", network_name))?;

        let shutdown = CancellationToken::new();
        let mut handles = Vec::new();

        // Create adapters for this network
        let adapters: Vec<Box<dyn ProtocolAdapter>> = vec![
            // HTTP adapter
            Box::new({
                let bind_addr = format!("{}:{}", network.http.bind_address, network.http.bind_port)
                    .parse::<SocketAddr>()
                    .unwrap_or_else(|_| {
                        panic!("Invalid bind address or port for network {}", network_name)
                    });
                HttpAdapter::new(network_name.clone(), bind_addr)
            }),
            // DIMSE adapter
            Box::new(DimseAdapter::new(network_name.clone())),
        ];

        // Start each adapter
        for adapter in adapters {
            match adapter
                .start(config.clone(), shutdown.clone())
                .await
            {
                Ok(task_handle) => {
                    let summary = adapter.summary();
                    tracing::info!("🚀 Started {} for network '{}'", summary, network_name);
                    
                    handles.push(AdapterHandle {
                        network_name: network_name.clone(),
                        adapter_summary: summary,
                        task_handle,
                        shutdown_token: shutdown.clone(),
                    });
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to start {} for network '{}': {}",
                        adapter.summary(),
                        network_name,
                        e
                    );
                }
            }
        }

        // Store handles in registry
        let mut adapters = self.adapters.write().await;
        adapters.insert(network_name, handles);

        Ok(())
    }

    /// Stop all adapters for a network
    pub async fn stop_network(&self, network_name: &str) -> Result<()> {
        let mut adapters = self.adapters.write().await;
        
        if let Some(handles) = adapters.remove(network_name) {
            tracing::info!("⏳ Stopping adapters for network '{}'", network_name);
            
            // Trigger shutdown for all adapters
            for handle in &handles {
                handle.shutdown_token.cancel();
            }
            
            // Wait for all to complete
            for handle in handles {
                let _ = handle.task_handle.await;
                tracing::info!("✓ Stopped {} for network '{}'", handle.adapter_summary, network_name);
            }
        }

        Ok(())
    }

    /// Restart adapters for a network (stop + start)
    pub async fn restart_network(
        &self,
        network_name: String,
        config: Arc<Config>,
    ) -> Result<()> {
        self.stop_network(&network_name).await?;
        self.start_network(network_name, config).await?;
        Ok(())
    }

    /// Stop all adapters in the registry
    pub async fn stop_all(&self) -> Result<()> {
        let network_names: Vec<String> = {
            let adapters = self.adapters.read().await;
            adapters.keys().cloned().collect()
        };

        for network_name in network_names {
            self.stop_network(&network_name).await?;
        }

        Ok(())
    }

    /// Get list of running networks
    pub async fn get_running_networks(&self) -> Vec<String> {
        let adapters = self.adapters.read().await;
        adapters.keys().cloned().collect()
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_registry_creation() {
        let registry = AdapterRegistry::new();
        let networks = registry.get_running_networks().await;
        assert!(networks.is_empty());
    }
}
