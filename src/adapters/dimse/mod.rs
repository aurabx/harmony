pub mod query_provider;
mod status_mapper;

use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::models::protocol::Protocol;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Registry of started DIMSE SCPs to prevent duplicate listeners
/// Key format: "{local_aet}@{bind_addr}:{port}#{endpoint_name}"
static STARTED_SCP: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// DIMSE protocol adapter
/// 
/// Handles DICOM DIMSE protocol via SCP (Service Class Provider) listeners.
/// Supports C-FIND, C-MOVE, C-STORE, and C-ECHO operations.
pub struct DimseAdapter {
    /// Network name this adapter serves
    pub network_name: String,
}

impl DimseAdapter {
    /// Create a new DIMSE adapter for the given network
    pub fn new(network_name: impl Into<String>) -> Self {
        Self {
            network_name: network_name.into(),
        }
    }

    /// Register an SCP in the global registry
    fn register_scp(key: String) -> bool {
        let mut guard = STARTED_SCP.lock().expect("SCP registry poisoned");
        if guard.contains(&key) {
            false
        } else {
            guard.insert(key);
            true
        }
    }

    /// Unregister an SCP from the global registry
    fn unregister_scp(key: &str) {
        let mut guard = STARTED_SCP.lock().expect("SCP registry poisoned");
        guard.retain(|k| k != key);
    }
}

#[async_trait]
impl ProtocolAdapter for DimseAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::Dimse
    }

    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let network_name = self.network_name.clone();
        
        tracing::info!("Starting DIMSE adapter for network '{}'", network_name);

        // Get all DIMSE endpoints for this network from pipelines
        let mut scp_configs: Vec<(String, String, HashMap<String, serde_json::Value>)> = Vec::new();
        
        for (pipeline_name, pipeline_cfg) in &config.pipelines {
            // Check if this pipeline belongs to our network
            if !pipeline_cfg.networks.contains(&network_name) {
                continue;
            }
            
            // Check all endpoints in this pipeline
            for endpoint_name in &pipeline_cfg.endpoints {
                if let Some(endpoint) = config.endpoints.get(endpoint_name) {
                    // Check if this endpoint is DIMSE by service name
                    if endpoint.service == "dimse" {
                        scp_configs.push((
                            pipeline_name.clone(),
                            endpoint_name.clone(),
                            endpoint.options.clone().unwrap_or_default(),
                        ));
                    }
                }
            }
        }

        if scp_configs.is_empty() {
            tracing::warn!(
                "No DIMSE endpoints found for network '{}', adapter will be idle",
                network_name
            );
        }

        // Spawn task to manage SCPs
        let handle = tokio::spawn(async move {
            let mut scp_handles = Vec::new();

            // Start each SCP
            for (pipeline_name, endpoint_name, options) in scp_configs {
                match Self::start_scp(&pipeline_name, &endpoint_name, &options).await {
                    Ok(scp_handle) => {
                        scp_handles.push(scp_handle);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to start DIMSE SCP for pipeline '{}', endpoint '{}': {}",
                            pipeline_name,
                            endpoint_name,
                            e
                        );
                    }
                }
            }

            // Wait for shutdown signal
            shutdown.cancelled().await;
            tracing::info!("DIMSE adapter for network '{}' shutting down", network_name);

            // Wait for all SCPs to complete
            for handle in scp_handles {
                let _ = handle.await;
            }

            tracing::info!("DIMSE adapter for network '{}' shut down", network_name);
        });

        Ok(handle)
    }

    fn summary(&self) -> String {
        format!("DimseAdapter for network '{}'", self.network_name)
    }
}

impl DimseAdapter {
    /// Start a single DIMSE SCP for the given pipeline and endpoint
    async fn start_scp(
        pipeline_name: &str,
        endpoint_name: &str,
        options: &std::collections::HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<JoinHandle<()>> {
        use dimse::{DimseConfig, DEFAULT_DIMSE_PORT};
        use std::net::IpAddr;

        let local_aet = options
            .get("local_aet")
            .and_then(|v| v.as_str())
            .unwrap_or("HARMONY_SCP")
            .to_string();

        let bind_addr = options
            .get("bind_addr")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<IpAddr>().ok())
            .unwrap_or_else(|| IpAddr::from(std::net::Ipv4Addr::new(0, 0, 0, 0)));

        let port = options
            .get("port")
            .and_then(|v| v.as_u64())
            .map(|p| p as u16)
            .unwrap_or(DEFAULT_DIMSE_PORT);

        let key = format!("{}@{}:{}#{}", local_aet, bind_addr, port, endpoint_name);

        // Check if already started
        if !Self::register_scp(key.clone()) {
            tracing::info!(
                "DIMSE SCP '{}' at {}:{} already running, skipping duplicate",
                local_aet,
                bind_addr,
                port
            );
            // Return a no-op handle
            return Ok(tokio::spawn(async {}));
        }

        let mut dimse_config = DimseConfig {
            local_aet: local_aet.clone(),
            bind_addr,
            port,
            ..Default::default()
        };

        // Determine storage_dir: prefer options, else storage adapter, else ./tmp/dimse
        if let Some(dir) = options.get("storage_dir").and_then(|v| v.as_str()) {
            dimse_config.storage_dir = std::path::PathBuf::from(dir);
        } else if let Some(storage) = crate::globals::get_storage() {
            let p = storage
                .ensure_dir_str("dimse")
                .unwrap_or_else(|_| std::path::PathBuf::from("./tmp/dimse"));
            dimse_config.storage_dir = p;
        } else {
            dimse_config.storage_dir = std::path::PathBuf::from("./tmp/dimse");
        }

        // Feature toggles
        if let Some(b) = options.get("enable_echo").and_then(|v| v.as_bool()) {
            dimse_config.enable_echo = b;
        }
        if let Some(b) = options.get("enable_find").and_then(|v| v.as_bool()) {
            dimse_config.enable_find = b;
        }
        if let Some(b) = options.get("enable_move").and_then(|v| v.as_bool()) {
            dimse_config.enable_move = b;
        }

        let pipeline = pipeline_name.to_string();
        let endpoint = endpoint_name.to_string();

        // Determine if we should use DCMTK storescp or internal SCP
        let is_persistent_backend = options
            .get("persistent_store_scp")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let use_dcmtk_store = options
            .get("use_dcmtk_store")
            .and_then(|v| v.as_bool())
            .unwrap_or(is_persistent_backend);

        if use_dcmtk_store {
            // Spawn DCMTK storescp process
            Self::start_dcmtk_scp(key, local_aet, port, dimse_config, pipeline, endpoint).await
        } else {
            // Use internal SCP with pipeline query provider
            Self::start_internal_scp(key, local_aet, bind_addr, port, dimse_config, pipeline, endpoint).await
        }
    }

    /// Start DCMTK storescp process
    async fn start_dcmtk_scp(
        key: String,
        local_aet: String,
        port: u16,
        dimse_config: dimse::DimseConfig,
        pipeline: String,
        endpoint: String,
    ) -> anyhow::Result<JoinHandle<()>> {
        use tokio::process::Command;

        let storage_dir = dimse_config.storage_dir.clone();
        let bind_addr = dimse_config.bind_addr;

        let handle = tokio::spawn(async move {
            let _ = tokio::fs::create_dir_all(&storage_dir).await;

            // Try to start DCMTK storescp
            let mut cmd = Command::new("storescp");
            cmd.arg("-v")
                .arg("-od")
                .arg(storage_dir.to_string_lossy().to_string())
                .arg("-aet")
                .arg(local_aet.clone())
                .arg(port.to_string());

            tracing::info!(
                "Starting DCMTK storescp AET='{}' on :{} -> {}",
                local_aet,
                port,
                storage_dir.display()
            );

            match cmd.spawn() {
                Ok(mut child) => {
                    if let Err(e) = child.wait().await {
                        tracing::error!("storescp exited with error: {}", e);
                    } else {
                        tracing::info!("storescp exited");
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to spawn storescp: {} — falling back to internal SCP",
                        e
                    );
                    // Fallback to internal SCP
                    let provider: Arc<dyn dimse::scp::QueryProvider> =
                        Arc::new(query_provider::PipelineQueryProvider::new(pipeline, endpoint));
                    let scp = dimse::DimseScp::new(dimse_config, provider);
                    if let Err(e2) = scp.run().await {
                        tracing::error!("DIMSE SCP '{}' failed: {}", local_aet, e2);
                    } else {
                        tracing::info!("DIMSE SCP '{}' stopped gracefully", local_aet);
                    }
                }
            }

            Self::unregister_scp(&key);
        });

        // Readiness loop (best-effort): try to connect to the listening port
        Self::wait_for_scp_ready(bind_addr, port).await;

        Ok(handle)
    }

    /// Start internal DIMSE SCP with pipeline query provider
    async fn start_internal_scp(
        key: String,
        local_aet: String,
        bind_addr: std::net::IpAddr,
        port: u16,
        dimse_config: dimse::DimseConfig,
        pipeline: String,
        endpoint: String,
    ) -> anyhow::Result<JoinHandle<()>> {
        let handle = tokio::spawn(async move {
            let provider: Arc<dyn dimse::scp::QueryProvider> =
                Arc::new(query_provider::PipelineQueryProvider::new(pipeline, endpoint));
            let scp = dimse::DimseScp::new(dimse_config, provider);
            
            tracing::info!(
                "Starting internal DIMSE SCP AET='{}' on {}:{}",
                local_aet,
                bind_addr,
                port
            );

            if let Err(e) = scp.run().await {
                tracing::error!("DIMSE SCP '{}' failed: {}", local_aet, e);
            } else {
                tracing::info!("DIMSE SCP '{}' stopped gracefully", local_aet);
            }

            Self::unregister_scp(&key);
        });

        // Readiness loop
        Self::wait_for_scp_ready(bind_addr, port).await;

        Ok(handle)
    }

    /// Wait for SCP to be ready by attempting TCP connection
    async fn wait_for_scp_ready(bind_addr: std::net::IpAddr, port: u16) {
        let target = if bind_addr.is_unspecified() {
            format!("127.0.0.1:{}", port)
        } else {
            format!("{}:{}", bind_addr, port)
        };

        for _ in 0..40 {
            if std::net::TcpStream::connect(&target).is_ok() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config::Config;

    #[test]
    fn test_dimse_adapter_creation() {
        let adapter = DimseAdapter::new("test_network");
        assert_eq!(adapter.network_name, "test_network");
        assert_eq!(adapter.protocol(), Protocol::Dimse);
    }

    #[test]
    fn test_dimse_adapter_summary() {
        let adapter = DimseAdapter::new("dimse_net");
        assert_eq!(adapter.summary(), "DimseAdapter for network 'dimse_net'");
    }

    #[tokio::test]
    async fn test_dimse_adapter_lifecycle() {
        let adapter = DimseAdapter::new("test_network");
        let config = Arc::new(Config::default());
        let shutdown = CancellationToken::new();

        // Start adapter (should complete without error even with empty config)
        let handle = adapter.start(config, shutdown.clone()).await.unwrap();

        // Trigger shutdown
        shutdown.cancel();

        // Wait for adapter to shut down
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Adapter should shut down gracefully");
    }

    #[test]
    fn test_dimse_adapter_is_protocol_adapter() {
        // Ensure DimseAdapter can be used as trait object
        let adapter: Box<dyn ProtocolAdapter> = Box::new(DimseAdapter::new("test"));
        assert_eq!(adapter.protocol(), Protocol::Dimse);
    }

    #[test]
    fn test_scp_registry() {
        // Clear registry for test isolation
        {
            let mut guard = STARTED_SCP.lock().unwrap();
            guard.clear();
        }

        let key1 = "TEST_AET@0.0.0.0:11112#endpoint1".to_string();
        let key2 = "TEST_AET@0.0.0.0:11113#endpoint2".to_string();

        // First registration should succeed
        assert!(DimseAdapter::register_scp(key1.clone()));
        
        // Duplicate registration should fail
        assert!(!DimseAdapter::register_scp(key1.clone()));
        
        // Different key should succeed
        assert!(DimseAdapter::register_scp(key2.clone()));

        // Unregister and re-register should succeed
        DimseAdapter::unregister_scp(&key1);
        assert!(DimseAdapter::register_scp(key1.clone()));

        // Cleanup
        DimseAdapter::unregister_scp(&key1);
        DimseAdapter::unregister_scp(&key2);
    }
}
