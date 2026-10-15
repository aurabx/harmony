use super::{dimse::DimseAdapter, http::HttpAdapter, http3::Http3Adapter, ProtocolAdapter};
use crate::config::config::Config;
use crate::models::protocol::Protocol;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
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

    /// Determine which protocols are required for a given network based on configured services
    ///
    /// Returns a HashSet of protocols that need adapters for this network,
    /// and logs any endpoint or service that could not be resolved.
    fn determine_required_protocols(network_name: &str, config: &Config) -> HashSet<Protocol> {
        let (required_protocols, problems) = Self::required_protocols(network_name, config);
        for problem in &problems {
            tracing::warn!("{}", problem);
        }

        let has_pipelines = config
            .pipelines
            .values()
            .any(|p| p.networks.iter().any(|n| n == network_name));
        if !has_pipelines {
            tracing::warn!("Network '{}' has no pipelines configured", network_name);
        } else if required_protocols.is_empty() {
            tracing::warn!(
                "Network '{}' has pipelines but no recognized service types that require protocol adapters",
                network_name
            );
        }

        required_protocols
    }

    /// The protocols `network_name` needs adapters for, with a message for
    /// each endpoint or service that could not be resolved.
    ///
    /// Scans every pipeline on the network and maps its endpoints' services
    /// to protocols. Does not log, so a reload can compare old and new
    /// configs without repeating warnings.
    pub(crate) fn required_protocols(
        network_name: &str,
        config: &Config,
    ) -> (HashSet<Protocol>, Vec<String>) {
        let mut required_protocols = HashSet::new();
        let mut problems = Vec::new();

        let pipelines = config
            .pipelines
            .iter()
            .filter(|(_, p)| p.networks.iter().any(|n| n == network_name));
        for (pipeline_name, pipeline_cfg) in pipelines {
            for endpoint_name in &pipeline_cfg.endpoints {
                let Some(endpoint) = config.endpoints.get(endpoint_name) else {
                    problems.push(format!(
                        "Endpoint '{}' referenced in pipeline '{}' not found in config",
                        endpoint_name, pipeline_name
                    ));
                    continue;
                };
                match crate::models::services::services::resolve_service(&endpoint.service) {
                    Ok(service) => {
                        required_protocols.insert(service.required_protocol());
                    }
                    Err(e) => problems.push(format!(
                        "Failed to resolve service '{}' in endpoint '{}': {}",
                        endpoint.service, endpoint_name, e
                    )),
                }
            }
        }

        (required_protocols, problems)
    }

    /// Start all adapters for a network
    pub async fn start_network(&self, network_name: String, config: Arc<Config>) -> Result<()> {
        let network = config
            .network
            .get(&network_name)
            .ok_or_else(|| anyhow::anyhow!("Network '{}' not found", network_name))?;

        let shutdown = CancellationToken::new();
        let mut handles = Vec::new();

        // Determine which protocols are required for this network
        let required_protocols = Self::determine_required_protocols(&network_name, &config);

        if required_protocols.is_empty() {
            tracing::warn!(
                "Network '{}' has no protocol adapters to start (no recognized service types found)",
                network_name
            );
            // Store empty handle set in registry
            let mut adapters = self.adapters.write().await;
            adapters.insert(network_name, handles);
            return Ok(());
        }

        // Log which protocols are being started
        let protocol_list: Vec<String> = required_protocols
            .iter()
            .map(|p| format!("{:?}", p))
            .collect();
        tracing::info!(
            "Starting protocol adapters for network '{}': [{}]",
            network_name,
            protocol_list.join(", ")
        );

        // Create adapters based on required protocols
        // Each adapter is responsible for extracting what it needs from the network config
        let mut adapters_to_start: Vec<Box<dyn ProtocolAdapter>> = Vec::new();

        for protocol in &required_protocols {
            match protocol {
                Protocol::Http => {
                    // Start TCP HTTP adapter if TCP config is present
                    if network.tcp_config.is_some() {
                        let adapter: Box<dyn ProtocolAdapter> =
                            HttpAdapter::from_network(network_name.clone(), network);
                        adapters_to_start.push(adapter);
                    } else {
                        tracing::info!(
                            "No TCP HTTP config for network '{}' (no [network.{}.http] / tcp_config); skipping HttpAdapter",
                            network_name,
                            network_name
                        );
                    }

                    // Start HTTP/3 adapter if configured
                    if network.http3.is_some() {
                        let adapter: Box<dyn ProtocolAdapter> =
                            Http3Adapter::from_network(network_name.clone(), network);
                        adapters_to_start.push(adapter);
                    }
                }
                Protocol::Dimse => {
                    let adapter: Box<dyn ProtocolAdapter> =
                        DimseAdapter::from_network(network_name.clone(), network);
                    adapters_to_start.push(adapter);
                }
                _ => {
                    tracing::warn!(
                        "No adapter implementation for protocol {:?}, skipping",
                        protocol
                    );
                    continue;
                }
            }
        }

        // Start each adapter
        for adapter in adapters_to_start {
            match adapter.start(config.clone(), shutdown.clone()).await {
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
                tracing::info!(
                    "✓ Stopped {} for network '{}'",
                    handle.adapter_summary,
                    network_name
                );
            }
        }

        Ok(())
    }

    /// Restart adapters for a network (stop + start)
    pub async fn restart_network(&self, network_name: String, config: Arc<Config>) -> Result<()> {
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
