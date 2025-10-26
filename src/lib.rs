pub mod adapters;
pub mod config;
mod file;
pub mod globals;
pub mod integrations;
pub mod models;
pub mod pipeline;
pub mod router;
pub mod storage;
pub mod storage_adapter;
mod utils;

use crate::adapters::dimse::DimseAdapter;
use crate::adapters::http::HttpAdapter;
use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::storage::create_storage_backend;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{self, prelude::*};

pub async fn run(config: Config) {
    let config = Arc::new(config);
    crate::globals::set_config(config.clone());

    // Initialize storage
    let storage =
        create_storage_backend(&config.storage).expect("Failed to create storage backend");
    crate::globals::set_storage(storage);

    // Initialise logging
    if config.logging.log_to_file {
        let file_appender = tracing_subscriber::fmt::layer()
            .with_file(true)
            .with_line_number(true)
            .with_writer(std::fs::File::create(&config.logging.log_file_path).unwrap());

        let stdout_appender = tracing_subscriber::fmt::layer()
            .with_file(true)
            .with_line_number(true);

        tracing_subscriber::registry()
            .with(file_appender)
            .with(stdout_appender)
            .try_init()
            .expect("Failed to initialise logging");
    } else {
        tracing_subscriber::fmt()
            .with_file(true)
            .with_line_number(true)
            .init();
    }

    tracing::info!("🔧 Starting Harmony '{}'", config.proxy.id);

    // Create shared shutdown token
    let shutdown = CancellationToken::new();
    let mut adapter_handles = Vec::new();

    // Start protocol adapters for each network
    for (network_name, network) in config.network.clone() {
        let config_clone = Arc::clone(&config);
        let shutdown_clone = shutdown.clone();

        // Determine which adapters are needed for this network
        let (needs_http, needs_dimse) = check_network_protocols(&config, &network_name);

        let mut adapters: Vec<Box<dyn ProtocolAdapter>> = Vec::new();

        // Only create HTTP adapter if there are HTTP endpoints
        if needs_http {
            let bind_addr = format!("{}:{}", network.http.bind_address, network.http.bind_port)
                .parse::<SocketAddr>()
                .unwrap_or_else(|_| {
                    panic!("Invalid bind address or port for network {}", network_name)
                });
            adapters.push(Box::new(HttpAdapter::new(network_name.clone(), bind_addr)));
        }

        // Only create DIMSE adapter if there are DICOM endpoints
        if needs_dimse {
            adapters.push(Box::new(DimseAdapter::new(network_name.clone())));
        }

        // Start each adapter
        for adapter in adapters {
            match adapter.start(config_clone.clone(), shutdown_clone.clone()).await {
                Ok(handle) => {
                    tracing::info!(
                        "🚀 Started {} for network '{}'",
                        adapter.summary(),
                        network_name
                    );
                    adapter_handles.push(handle);
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
    }

    // Wait for ctrl-c signal
    tracing::info!("✓ All adapters started. Press Ctrl+C to shutdown.");
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl-c signal");

    // Trigger shutdown
    tracing::info!("⏳ Shutting down...");
    shutdown.cancel();

    // Wait for all adapters to complete
    for handle in adapter_handles {
        let _ = handle.await;
    }

    tracing::info!("✓ Harmony shut down gracefully.");
}

/// Check which protocol adapters are needed for a given network
fn check_network_protocols(config: &Config, network_name: &str) -> (bool, bool) {
    let mut needs_http = false;
    let mut needs_dimse = false;

    for pipeline in config.pipelines.values() {
        // Skip pipelines not on this network
        if !pipeline.networks.contains(&network_name.to_string()) {
            continue;
        }

        // Check endpoints
        for endpoint_name in &pipeline.endpoints {
            if let Some(endpoint) = config.endpoints.get(endpoint_name) {
                match endpoint.service.as_str() {
                    "dicom_scp" => needs_dimse = true,
                    _ => needs_http = true,
                }
            }
        }

        // Check backends for legacy persistent DICOM SCPs
        for backend_name in &pipeline.backends {
            if let Some(backend) = config.backends.get(backend_name) {
                if backend.service == "dicom" {
                    if let Some(options) = &backend.options {
                        let needs_persistent = options
                            .get("persistent_store_scp")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                            || (options.contains_key("host") && options.contains_key("port"));

                        if needs_persistent && options.contains_key("incoming_store_port") {
                            needs_dimse = true;
                        }
                    }
                }
            }
        }
    }

    (needs_http, needs_dimse)
}
