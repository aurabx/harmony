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

use crate::adapters::registry::AdapterRegistry;
use crate::config::config::Config;
use crate::config::watcher::ConfigWatcher;
use crate::storage::create_storage_backend;
use std::sync::Arc;
use tracing_subscriber::{self, prelude::*};

pub async fn run(config: Config) {
    run_with_reload(config, None).await;
}

pub async fn run_with_reload(config: Config, config_path: Option<String>) {
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

    // Create adapter registry
    let registry = Arc::new(AdapterRegistry::new());

    // Start protocol adapters for each network
    for network_name in config.network.keys() {
        registry
            .start_network(network_name.clone(), config.clone())
            .await
            .expect("Failed to start network");
    }

    tracing::info!("✓ All adapters started. Press Ctrl+C to shutdown.");

    // Start config watcher if config path provided
    if let Some(path) = config_path {
        let watcher = ConfigWatcher::new(path, registry.clone());
        tokio::spawn(async move {
            if let Err(e) = watcher.start().await {
                tracing::error!("Config watcher error: {}", e);
            }
        });
    }

    // Wait for ctrl-c signal
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl-c signal");

    // Trigger shutdown
    tracing::info!("⏳ Shutting down...");
    registry.stop_all().await.expect("Failed to stop adapters");

    tracing::info!("✓ Harmony shut down gracefully.");
}
