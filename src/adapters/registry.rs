use super::{dimse::DimseAdapter, http::HttpAdapter, http3::Http3Adapter, ProtocolAdapter};
use crate::config::config::Config;
use crate::models::protocol::Protocol;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// How long stop_network waits for an adapter task before aborting it.
/// The plain-HTTP adapter's graceful shutdown waits for every in-flight
/// request, so a slow client could otherwise stall a reload forever.
const ADAPTER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

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
    shutdown_timeout: Duration,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: Arc::new(RwLock::new(HashMap::new())),
            shutdown_timeout: ADAPTER_SHUTDOWN_TIMEOUT,
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
    ///
    /// The network's handles are taken out of the registry before waiting, so
    /// the registry lock is not held while adapters drain. Each adapter gets
    /// until a shared deadline (`shutdown_timeout` from now) to finish; any
    /// still running then is aborted.
    pub async fn stop_network(&self, network_name: &str) -> Result<()> {
        let Some(handles) = self.adapters.write().await.remove(network_name) else {
            return Ok(());
        };
        self.stop_handles(vec![(network_name.to_string(), handles)]).await;
        Ok(())
    }

    /// Cancels every given adapter at once, then waits for all of them
    /// against one deadline, so stopping several networks takes at most one
    /// `shutdown_timeout` in total.
    async fn stop_handles(&self, networks: Vec<(String, Vec<AdapterHandle>)>) {
        for (network_name, handles) in &networks {
            tracing::info!("⏳ Stopping adapters for network '{}'", network_name);
            for handle in handles {
                handle.shutdown_token.cancel();
            }
        }

        let deadline = tokio::time::Instant::now() + self.shutdown_timeout;
        for (network_name, handles) in networks {
            for handle in handles {
                Self::await_adapter_until(handle, deadline, &network_name).await;
            }
        }
    }

    /// Waits for one adapter task until `deadline`, then aborts it. Awaits
    /// the aborted task too, so the adapter's own listener is closed before a
    /// restart tries to bind it. Connection tasks the adapter spawned are not
    /// children of this task; see AURA-2515 for plain HTTP.
    async fn await_adapter_until(
        handle: AdapterHandle,
        deadline: tokio::time::Instant,
        network_name: &str,
    ) {
        let AdapterHandle {
            adapter_summary,
            mut task_handle,
            ..
        } = handle;

        match tokio::time::timeout_at(deadline, &mut task_handle).await {
            Ok(Ok(())) => {
                tracing::info!("✓ Stopped {} for network '{}'", adapter_summary, network_name);
            }
            Ok(Err(e)) => {
                tracing::error!(
                    "{} for network '{}' ended with an error: {}",
                    adapter_summary,
                    network_name,
                    e
                );
            }
            Err(_) => {
                tracing::warn!(
                    "{} for network '{}' did not stop within the shutdown timeout; aborting it",
                    adapter_summary,
                    network_name
                );
                task_handle.abort();
                // The JoinError here only reports the cancellation just requested.
                let _ = task_handle.await;
            }
        }
    }

    /// Restart adapters for a network (stop + start)
    pub async fn restart_network(&self, network_name: String, config: Arc<Config>) -> Result<()> {
        self.stop_network(&network_name).await?;
        self.start_network(network_name, config).await?;
        Ok(())
    }

    /// Stop all adapters in the registry
    pub async fn stop_all(&self) -> Result<()> {
        let networks: Vec<(String, Vec<AdapterHandle>)> =
            self.adapters.write().await.drain().collect();
        self.stop_handles(networks).await;
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

    /// An adapter handle whose task ignores cancellation and never ends.
    fn handle_that_never_stops(network_name: &str) -> AdapterHandle {
        AdapterHandle {
            network_name: network_name.to_string(),
            adapter_summary: "stuck adapter".to_string(),
            task_handle: tokio::spawn(std::future::pending::<()>()),
            shutdown_token: CancellationToken::new(),
        }
    }

    /// stop_all must share one deadline across networks. Waiting one full
    /// timeout per network could outlast a supervisor's shutdown grace period.
    #[tokio::test]
    async fn stop_all_uses_one_deadline_for_all_networks() {
        let mut registry = AdapterRegistry::new();
        registry.shutdown_timeout = Duration::from_millis(300);
        {
            let mut adapters = registry.adapters.write().await;
            for name in ["a", "b", "c"] {
                adapters.insert(name.to_string(), vec![handle_that_never_stops(name)]);
            }
        }

        let started = tokio::time::Instant::now();
        registry.stop_all().await.unwrap();
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_millis(600),
            "stop_all took {:?}; three networks were waited on one after another",
            elapsed
        );
        assert!(registry.get_running_networks().await.is_empty());
    }

    /// A plain-HTTP server with graceful shutdown, like the HTTP adapter's,
    /// whose only route never answers. Returns the bound address and the
    /// server task.
    async fn spawn_server_with_stuck_route(
        shutdown: CancellationToken,
    ) -> (std::net::SocketAddr, JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/stuck",
            axum::routing::get(|| std::future::pending::<&'static str>()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move { shutdown.cancelled().await })
                .await;
        });
        (addr, task)
    }

    /// Opens a connection and sends a request to the stuck route. The
    /// returned stream must be kept alive to hold the request in flight.
    async fn hold_stuck_request(addr: std::net::SocketAddr) -> tokio::net::TcpStream {
        use tokio::io::AsyncWriteExt;
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /stuck HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        // Give the server time to accept the connection and start the handler.
        tokio::time::sleep(Duration::from_millis(100)).await;
        stream
    }

    /// axum's graceful shutdown waits for every in-flight request. A request
    /// that never finishes must not stall stop_network past its timeout, and
    /// must not keep the registry locked while stop_network waits.
    #[tokio::test]
    async fn stop_network_does_not_wait_forever_for_stuck_connection() {
        let mut registry = AdapterRegistry::new();
        registry.shutdown_timeout = Duration::from_millis(300);

        let shutdown = CancellationToken::new();
        let (addr, task) = spawn_server_with_stuck_route(shutdown.clone()).await;
        registry.adapters.write().await.insert(
            "stuck".to_string(),
            vec![AdapterHandle {
                network_name: "stuck".to_string(),
                adapter_summary: "test server".to_string(),
                task_handle: task,
                shutdown_token: shutdown,
            }],
        );
        let _client = hold_stuck_request(addr).await;

        let registry = Arc::new(registry);
        let stopping = {
            let registry = registry.clone();
            tokio::spawn(async move { registry.stop_network("stuck").await })
        };

        // The registry must stay readable while the stop is in progress.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let running = tokio::time::timeout(Duration::from_millis(100), registry.get_running_networks())
            .await
            .expect("registry lock was held while stop_network waited");
        assert!(running.is_empty());

        tokio::time::timeout(Duration::from_secs(2), stopping)
            .await
            .expect("stop_network did not return after its timeout")
            .unwrap()
            .unwrap();

        // The listener is gone, so a restart could bind the port again.
        tokio::net::TcpListener::bind(addr)
            .await
            .expect("port still bound after stop_network");
    }
}
