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
use runbeam_sdk::load_token;
use std::sync::Arc;
use tracing_subscriber::{self, prelude::*};

pub async fn run(config: Config) {
    run_with_reload(config, None).await;
}

pub async fn run_with_reload(config: Config, config_path: Option<String>) {
    let config = Arc::new(config);
    crate::globals::set_config(config.clone());

    // Set config path for management API if provided
    if let Some(ref path) = config_path {
        crate::globals::set_config_path(path.clone());
    }

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

    // Set global registry for management API
    crate::globals::set_adapter_registry(registry.clone());

    // Start protocol adapters for each network
    for network_name in config.network.keys() {
        registry
            .start_network(network_name.clone(), config.clone())
            .await
            .expect("Failed to start network");
    }

    tracing::info!("✓ All adapters started. Press Ctrl+C to shutdown.");

    // Start config watcher if config path provided
    let watcher_shutdown = tokio_util::sync::CancellationToken::new();
    if let Some(path) = config_path {
        let watcher = ConfigWatcher::new(path, registry.clone());
        let shutdown_clone = watcher_shutdown.clone();
        tokio::spawn(async move {
            tokio::select! {
                result = watcher.start() => {
                    if let Err(e) = result {
                        tracing::error!("Config watcher error: {}", e);
                    }
                }
                _ = shutdown_clone.cancelled() => {
                    tracing::info!("Config watcher stopped");
                }
            }
        });
    }

    // Check for existing machine token and start cloud polling if valid
    // Get proxy ID for instance isolation
    let proxy_id = config.proxy.id.clone();

    // Check for machine token from environment variable first (for headless/pre-provisioned deployments)
    let token_from_env = std::env::var("RUNBEAM_MACHINE_TOKEN").ok().and_then(|token_str| {
        // Try to parse as JSON
        match serde_json::from_str::<runbeam_sdk::MachineToken>(&token_str) {
            Ok(token) => {
                tracing::info!("Using machine token from RUNBEAM_MACHINE_TOKEN environment variable");
                Some(token)
            }
            Err(e) => {
                tracing::warn!("Failed to parse RUNBEAM_MACHINE_TOKEN: {}. Falling back to stored token.", e);
                None
            }
        }
    });

    // Try environment variable first, then fall back to secure storage
    let token_result = if let Some(token) = token_from_env {
        Ok(Some(token))
    } else {
        // Try to load existing machine token from secure storage (SDK manages keyring/encrypted filesystem)
        load_token(&proxy_id, "auth").await
    };

    match token_result {
            Ok(Some(token)) if token.is_valid() => {
                // Valid token found - start cloud polling
                let poll_interval = config.management.poll_interval();
                let base_url = config.management.cloud_api_base_url.clone()
                    .unwrap_or_else(|| "https://api.runbeam.cloud".to_string());

                tracing::info!(
                    "🌥️  Found valid stored token (gateway: {}), starting cloud polling",
                    token.gateway_code
                );

                let cloud_shutdown = tokio_util::sync::CancellationToken::new();
                crate::globals::set_cloud_polling_token(cloud_shutdown.clone());

                let client = runbeam_sdk::RunbeamClient::new(base_url);
                let registry_clone = registry.clone();

                tokio::spawn(async move {
                    crate::models::services::types::management::cloud_poller::start_cloud_polling(
                        client,
                        token.machine_token,
                        poll_interval,
                        registry_clone,
                        cloud_shutdown,
                    )
                    .await;
                });
            }
            Ok(Some(token)) => {
                tracing::warn!(
                    "Stored token for gateway '{}' has expired (expired at: {}). Waiting for re-authorization.",
                    token.gateway_code,
                    token.expires_at
                );
            }
            Ok(None) => {
                tracing::info!("No stored token found. Gateway must be authorized via /admin/authorize endpoint.");
            }
        Err(e) => {
            tracing::warn!("Failed to load stored token: {}. Waiting for authorization.", e);
        }
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
