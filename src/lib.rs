pub mod adapters;
pub mod clients;
pub mod config;
pub mod core;
mod file;
pub mod globals;
pub mod integrations;
pub mod models;
pub mod pipeline;
pub mod router;
pub mod storage;
pub mod storage_adapter;
mod utils;
pub mod management;

use crate::adapters::registry::AdapterRegistry;
use crate::config::config::Config;
use crate::config::watcher::ConfigWatcher;
use crate::integrations::provider_resolver::ProviderResolver;
use crate::storage::create_storage_backend;
use runbeam_sdk::{load_token, save_token};
use std::path::Path;
use std::sync::Arc;
use tracing_subscriber::{self, prelude::*};

pub async fn run(config: Config) {
    run_with_reload(config, None).await;
}

pub async fn run_with_reload(config: Config, config_path: Option<String>) {
    // Install the rustls crypto provider. This is required because multiple crypto
    // providers (ring, aws-lc-rs) are present in the dependency tree via different
    // crates, so rustls cannot auto-select one. We use aws-lc-rs as it's already
    // a transitive dependency and avoids pulling in BoringSSL via ring.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Initialize provider resolver from config providers
    // MeshRegistry will use this to resolve remote references when building
    let resolver = Arc::new(ProviderResolver::new(config.provider.clone()));
    crate::globals::set_provider_resolver(resolver);

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
    // Use try_init to allow multiple initializations in test environments
    if config.logging.log_to_file {
        let file_appender = tracing_subscriber::fmt::layer()
            .with_file(true)
            .with_line_number(true)
            .with_writer(std::fs::File::create(&config.logging.log_file_path).unwrap());

        let stdout_appender = tracing_subscriber::fmt::layer()
            .with_file(true)
            .with_line_number(true);

        let _ = tracing_subscriber::registry()
            .with(file_appender)
            .with(stdout_appender)
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_file(true)
            .with_line_number(true)
            .try_init();
    }

    tracing::info!("🔧 Starting Harmony '{}'", config.proxy.effective_id());

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
        // Resolve pipelines directory path relative to config file
        let config_dir = Path::new(&path)
            .parent()
            .expect("Failed to get config file directory");
        let pipelines_dir = config_dir.join(&config.proxy.pipelines_path);
        let pipelines_path = if pipelines_dir.exists() {
            Some(pipelines_dir.to_string_lossy().to_string())
        } else {
            None
        };

        let watcher = ConfigWatcher::new(path, pipelines_path, registry.clone());
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

    // Check for cloud integration via primary provider
    if config.is_cloud_enabled() {
        // Get proxy ID for instance isolation
        let proxy_id = config.proxy.effective_id().to_string();

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
        let token_result = if let Some(ref token) = token_from_env {
            // Save env token to storage so management API handlers can access it
            if let Err(e) = save_token(&proxy_id, "auth", token).await {
                tracing::warn!("Failed to save env token to storage: {}", e);
            }
            Ok(Some(token.clone()))
        } else {
            // Try to load existing machine token from secure storage (SDK manages keyring/encrypted filesystem)
            load_token(&proxy_id, "auth").await
        };

        match token_result {
            Ok(Some(token)) if token.is_valid() => {
                // Valid token found - start cloud polling using primary provider settings
                let poll_interval = config.primary_poll_interval()
                    .unwrap_or_else(|| std::time::Duration::from_secs(30));
                let base_url = config.primary_api_base_url();

                tracing::info!(
                    "🌥️  Found valid stored token (gateway: {}), starting cloud polling",
                    token.gateway_id
                );

                let cloud_shutdown = tokio_util::sync::CancellationToken::new();
                crate::globals::set_cloud_polling_token(cloud_shutdown.clone());

                let initial_client = runbeam_sdk::RunbeamClient::new(base_url);
                let registry_clone = registry.clone();
                let machine_token = token.machine_token.clone();

                // Check if push-on-startup is enabled (default: off)
                let push_config_on_startup = std::env::var("RUNBEAM_PUSH_CONFIG_ON_STARTUP")
                    .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                    .unwrap_or(false);

                tokio::spawn(async move {
                    // Discover actual API base URL before starting poller
                    let client = match initial_client.discover_base_url(&machine_token).await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(
                                "Base URL discovery failed (using configured URL): {}",
                                e
                            );
                            initial_client
                        }
                    };

                    // Check if push-on-startup is enabled (default: off)
                    if push_config_on_startup {
                        // Start polling with ready signal to prevent race conditions
                        let ready_signal = management::cloud_poller::start_cloud_polling_when_ready(
                            client.clone(),
                            machine_token.clone(),
                            poll_interval,
                            registry_clone.clone(),
                            cloud_shutdown.clone(),
                        ).await;

                        // Push config while polling is waiting
                        // Note: After push completes, Runbeam Cloud should create Change records for the
                        // pushed configs, which will then be picked up by the normal polling loop.
                        // This ensures the gateway gets cloud-assigned IDs and stays in sync.
                        if let Err(e) = management::cloud_poller::push_config_on_startup(
                            &client,
                            &machine_token,
                        ).await {
                            tracing::warn!("Push config on startup failed: {}. Continuing with polling.", e);
                        }

                        // Signal polling to start and trigger immediate poll
                        let _ = ready_signal.send(());
                        if crate::globals::trigger_cloud_poll() {
                            tracing::info!("Triggered immediate cloud poll after startup config push");
                        }
                    } else {
                        // Start polling immediately if no startup push
                        management::cloud_poller::start_cloud_polling(
                            client,
                            machine_token,
                            poll_interval,
                            registry_clone,
                            cloud_shutdown,
                        )
                        .await;
                    }
                });
            }
            Ok(Some(token)) => {
                tracing::warn!(
                        "Stored token for gateway '{}' has expired (expired at: {}). Waiting for re-authorization.",
                        token.gateway_id,
                        token.expires_at
                    );
            }
            Ok(None) => {
                tracing::info!("No stored token found. Gateway must be authorized via /admin/authorize endpoint.");
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to load stored token: {}. Waiting for authorization.",
                    e
                );
            }
        }
    } else {
        tracing::info!(
            "Cloud integration is disabled (primary_provider: {}, not enabled or no API configured)",
            config.proxy.primary_provider
        );
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
