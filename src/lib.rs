pub mod adapters;
pub mod config;
mod file;
pub mod globals;
pub mod integrations;
pub mod models;
pub mod pipeline;
pub mod router;
pub mod storage;
mod utils;

use crate::adapters::dimse::DimseAdapter;
use crate::adapters::http::HttpAdapter;
use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::storage::create_storage_backend;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{self, prelude::*};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PersistentScpSpec {
    pub backend_name: String,
    pub bind_addr: IpAddr,
    pub port: u16,
    pub local_aet: String,
    pub storage_dir: PathBuf,
    pub enable_echo: bool,
    pub enable_find: bool,
    pub enable_move: bool,
}

pub fn collect_required_dimse_scps(config: &Config) -> Vec<PersistentScpSpec> {
    let mut specs = Vec::new();
    
    for (backend_name, backend) in &config.backends {
        // Only check DICOM backends
        if backend.service != "dicom" {
            continue;
        }
        
        let options = match &backend.options {
            Some(opts) => opts,
            None => continue,
        };
        
        // Check if persistent Store SCP is required
        // Either explicitly requested OR auto-detected for DICOM backends that could use C-MOVE
        let explicit_persistent_scp = options
            .get("persistent_store_scp")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
            
        // Auto-detect: if this is a DICOM backend with remote host/port, it likely needs SCP for C-MOVE
        let has_remote_connection = options.contains_key("host") && options.contains_key("port");
        let needs_persistent_scp = explicit_persistent_scp || has_remote_connection;
            
        if !needs_persistent_scp {
            continue;
        }
        
        // Get or assign incoming_store_port
        let port = match options.get("incoming_store_port").and_then(|v| v.as_u64()) {
            Some(p) if (1..=65535).contains(&p) => p as u16,
            Some(p) => {
                tracing::warn!(
                    "Backend '{}' has invalid incoming_store_port={}, skipping SCP",
                    backend_name, p
                );
                continue;
            }
            None => {
                // Auto-assign a port if not explicitly set
                if explicit_persistent_scp {
                    tracing::warn!(
                        "Backend '{}' has persistent_store_scp=true but missing incoming_store_port, skipping SCP",
                        backend_name
                    );
                    continue;
                } else {
                    // For auto-detected backends, use a reasonable default port
                    // Use the remote port + 10000 to avoid conflicts, or 11112 as fallback
                    if let Some(remote_port) = options.get("port").and_then(|v| v.as_u64()) {
                        let suggested_port = (remote_port + 10000).min(65535) as u16;
                        if suggested_port > 1024 { // Avoid privileged ports
                            tracing::debug!(
                                "Auto-assigning incoming_store_port={} for backend '{}' (remote_port + 10000)",
                                suggested_port, backend_name
                            );
                            suggested_port
                        } else {
                            11112 // Safe default
                        }
                    } else {
                        11112 // Safe default
                    }
                }
            }
        };
        
        // Parse bind_addr (default "0.0.0.0")
        let bind_addr = options
            .get("bind_addr")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0.0")
            .parse::<IpAddr>()
            .unwrap_or_else(|_| {
                tracing::warn!(
                    "Backend '{}' has invalid bind_addr, using default 0.0.0.0",
                    backend_name
                );
                "0.0.0.0".parse().unwrap()
            });
            
        // Parse local_aet (default "HARMONY_SCU")
        let local_aet = options
            .get("local_aet")
            .and_then(|v| v.as_str())
            .unwrap_or("HARMONY_SCU")
            .to_string();
            
        // Parse storage_dir (default "./tmp/dimse")
        let storage_dir = options
            .get("storage_dir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./tmp/dimse"));
            
        // Parse optional feature flags
        let enable_echo = options
            .get("enable_echo")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let enable_find = options
            .get("enable_find")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let enable_move = options
            .get("enable_move")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
            
        specs.push(PersistentScpSpec {
            backend_name: backend_name.clone(),
            bind_addr,
            port,
            local_aet,
            storage_dir,
            enable_echo,
            enable_find,
            enable_move,
        });
        
        tracing::debug!(
            "Found DICOM backend '{}' requiring persistent SCP on {}:{}",
            backend_name, bind_addr, port
        );
    }
    
    specs
}

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
        let network_name_clone = network_name.clone();
        let shutdown_clone = shutdown.clone();

        // Parse bind address for HTTP
        let bind_addr = format!("{}:{}", network.http.bind_address, network.http.bind_port)
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| {
                panic!("Invalid bind address or port for network {}", network_name)
            });
        
        // Start HTTP adapter
        let http_adapter = HttpAdapter::new(network_name_clone.clone(), bind_addr);
        
        match http_adapter.start(config_clone.clone(), shutdown_clone.clone()).await {
            Ok(handle) => {
                tracing::info!(
                    "🚀 Started HTTP adapter for network '{}'",
                    network_name
                );
                adapter_handles.push(handle);
            }
            Err(e) => {
                tracing::error!(
                    "Failed to start HTTP adapter for network '{}': {}",
                    network_name,
                    e
                );
            }
        }

        // Check if network needs DIMSE services
        let has_dimse_endpoint = config.pipelines.values().any(|pipeline| {
            pipeline.networks.contains(&network_name)
                && pipeline.endpoints.iter().any(|endpoint_name| {
                    config
                        .endpoints
                        .get(endpoint_name)
                        .map(|e| e.service == "dimse")
                        .unwrap_or(false)
                })
        });
        
        // Collect required persistent DICOM SCPs for this network
        let scp_specs: Vec<_> = collect_required_dimse_scps(&config)
            .into_iter()
            .filter(|spec| {
                // Filter SCPs for pipelines using this network
                config.pipelines.values().any(|pipeline| {
                    pipeline.networks.contains(&network_name)
                        && pipeline.backends.iter().any(|backend_name| {
                            backend_name == &spec.backend_name
                        })
                })
            })
            .collect();
            
        let needs_any_dimse = has_dimse_endpoint || !scp_specs.is_empty();

        if needs_any_dimse {
            // Start existing DIMSE adapter for endpoints
            if has_dimse_endpoint {
                let dimse_adapter = DimseAdapter::new(network_name_clone.clone());
                match dimse_adapter.start(config_clone.clone(), shutdown_clone.clone()).await {
                    Ok(handle) => {
                        tracing::info!(
                            "🚀 Started DIMSE adapter for network '{}'",
                            network_name
                        );
                        adapter_handles.push(handle);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to start DIMSE adapter for network '{}': {}",
                            network_name,
                            e
                        );
                    }
                }
            }
            
            // Start persistent DICOM SCPs
            // Deduplicate by (bind_addr, port) to avoid conflicts
            let mut unique_listeners: std::collections::HashSet<(IpAddr, u16)> = std::collections::HashSet::new();
            for spec in scp_specs {
                let listener_key = (spec.bind_addr, spec.port);
                if !unique_listeners.insert(listener_key) {
                    tracing::debug!(
                        "Skipping duplicate SCP listener on {}:{} (backend '{}')",
                        spec.bind_addr, spec.port, spec.backend_name
                    );
                    continue;
                }
                
                let storage_dir = spec.storage_dir.clone();
                let backend_name = spec.backend_name.clone();
                let bind_addr = spec.bind_addr;
                let port = spec.port;
                let local_aet = spec.local_aet.clone();
                let enable_echo = spec.enable_echo;
                let enable_find = spec.enable_find;
                let enable_move = spec.enable_move;
                let shutdown_clone = shutdown_clone.clone();
                
                tracing::info!(
                    "🩺 Starting persistent DICOM SCP for backend '{}' on {}:{}, AE='{}', storage='{:?}'",
                    backend_name, bind_addr, port, local_aet, storage_dir
                );
                
                // Spawn persistent SCP task
                let scp_handle = tokio::spawn(async move {
                    // Ensure storage directory exists
                    if let Err(e) = tokio::fs::create_dir_all(&storage_dir).await {
                        tracing::error!(
                            "Failed to create storage directory {:?} for backend '{}': {}",
                            storage_dir, backend_name, e
                        );
                        return;
                    }
                    
                    let dimse_config = dimse::DimseConfig {
                        local_aet,
                        bind_addr,
                        port,
                        storage_dir,
                        enable_echo,
                        enable_find,
                        enable_move,
                        ..Default::default()
                    };
                    
                    // Use internal SCP for persistent listeners
                    let provider: Arc<dyn dimse::scp::QueryProvider> = Arc::new(
                        crate::adapters::dimse::query_provider::PipelineQueryProvider::new(
                            "persistent_scp".to_string(),
                            backend_name.clone()
                        )
                    );
                    let scp = dimse::DimseScp::new(dimse_config, provider);
                    
                    // Run SCP until shutdown signal
                    tokio::select! {
                        result = scp.run() => {
                            if let Err(e) = result {
                                tracing::error!("Persistent DICOM SCP '{}' failed: {}", backend_name, e);
                            } else {
                                tracing::info!("Persistent DICOM SCP '{}' stopped gracefully", backend_name);
                            }
                        }
                        _ = shutdown_clone.cancelled() => {
                            tracing::info!("Persistent DICOM SCP '{}' shutting down", backend_name);
                        }
                    }
                });
                
                adapter_handles.push(scp_handle);
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
