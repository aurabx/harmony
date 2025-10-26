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
        // Initialize tracing if RUST_LOG is set
        if std::env::var("RUST_LOG").is_ok() {
            let _ = tracing_subscriber::fmt()
                .with_test_writer()
                .try_init();
        }

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

        // Give the adapter time to start listeners and bind to ports
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        // Verify that SCP ports are actually listening
        self.wait_for_scp_ports().await;

        Ok(())
    }

    /// Wait for SCP ports to be listening
    async fn wait_for_scp_ports(&self) {
        // Extract ports from dicom_scp endpoints
        for (_pipeline_name, pipeline_cfg) in &self.config.pipelines {
            if !pipeline_cfg.networks.contains(&self.network_name) {
                continue;
            }

            for endpoint_name in &pipeline_cfg.endpoints {
                if let Some(endpoint) = self.config.endpoints.get(endpoint_name) {
                    if endpoint.service == "dicom_scp" {
                        if let Some(options) = &endpoint.options {
                            let port = options
                                .get("port")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(11112) as u16;

                            // Try to connect to verify the port is listening
                            for attempt in 0..50 {
                                match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                                    Ok(_) => {
                                        eprintln!("✓ SCP port {} ready after {} attempts", port, attempt + 1);
                                        break;
                                    }
                                    Err(e) if attempt == 49 => {
                                        eprintln!("⚠ SCP port {} not ready after 50 attempts: {}", port, e);
                                    }
                                    Err(_) => {
                                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
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
    #[allow(dead_code)]
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
