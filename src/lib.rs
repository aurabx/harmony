pub mod adapters;
pub mod config;
mod file;
pub mod globals;
pub mod integrations;
pub mod models;
pub mod pipeline;
pub mod router;
pub mod runbeam_api;
pub mod storage;
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

        // Create and start all adapters for this network
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
