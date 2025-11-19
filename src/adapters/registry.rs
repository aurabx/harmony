use super::{dimse::DimseAdapter, http::HttpAdapter, ProtocolAdapter};
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
    /// Returns a HashSet of protocols that need adapters for this network.
    /// Scans all pipelines that reference the network and maps their endpoint services to protocols.
    fn determine_required_protocols(network_name: &str, config: &Config) -> HashSet<Protocol> {
        let mut required_protocols = HashSet::new();
        let mut found_pipelines = Vec::new();

        // Find all pipelines that include this network
        for (pipeline_name, pipeline_cfg) in &config.pipelines {
            if pipeline_cfg.networks.contains(&network_name.to_string()) {
                found_pipelines.push(pipeline_name.as_str());
                tracing::debug!(
                    "Pipeline '{}' includes network '{}'",
                    pipeline_name,
                    network_name
                );

                // Examine all endpoints in this pipeline
                for endpoint_name in &pipeline_cfg.endpoints {
                    if let Some(endpoint) = config.endpoints.get(endpoint_name) {
                        let service_type = &endpoint.service;
                        tracing::debug!(
                            "Examining endpoint '{}' with service type '{}'",
                            endpoint_name,
                            service_type
                        );

                        // Get the protocol from the service definition
                        match crate::models::services::services::resolve_service(service_type) {
                            Ok(service) => {
                                let protocol = service.required_protocol();
                                tracing::debug!(
                                    "Service '{}' requires protocol {:?}",
                                    service_type,
                                    protocol
                                );
                                required_protocols.insert(protocol);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to resolve service '{}' in endpoint '{}': {}",
                                    service_type,
                                    endpoint_name,
                                    e
                                );
                            }
                        }
                    } else {
                        tracing::warn!(
                            "Endpoint '{}' referenced in pipeline '{}' not found in config",
                            endpoint_name,
                            pipeline_name
                        );
                    }
                }
            }
        }

        if found_pipelines.is_empty() {
            tracing::warn!("Network '{}' has no pipelines configured", network_name);
        }

        if required_protocols.is_empty() && !found_pipelines.is_empty() {
            tracing::warn!(
                "Network '{}' has pipelines but no recognized service types that require protocol adapters",
                network_name
            );
        }

        required_protocols
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
            let adapter: Box<dyn ProtocolAdapter> = match protocol {
                Protocol::Http => HttpAdapter::from_network(network_name.clone(), network),
                Protocol::Dimse => DimseAdapter::from_network(network_name.clone(), network),
                _ => {
                    tracing::warn!(
                        "No adapter implementation for protocol {:?}, skipping",
                        protocol
                    );
                    continue;
                }
            };
            adapters_to_start.push(adapter);
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
