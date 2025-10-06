pub mod config;
pub mod router;
pub mod models;
mod utils;
pub mod globals;
pub mod integrations;

use std::net::SocketAddr;
use std::sync::Arc;
use axum::serve;
use tokio::net::TcpListener;
use tracing_subscriber::{self, prelude::*};
use crate::router::{build_network_router};
use crate::config::config::Config;

pub async fn run(config: Config) {
    let config = Arc::new(config);
    crate::globals::set_config(config.clone());

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

    // Start servers for each network
    for (network_name, network) in config.network.clone() { // Clone `config.network` for proper ownership
        let config_clone = Arc::clone(&config); // Clone the Arc<Config> to ensure shared ownership
        let network_name = network_name.clone();
        let network = network.clone();

        tokio::spawn(async move {
            let base_app = build_network_router(config_clone.clone(), &network_name).await;
            
            let addr = format!("{}:{}", network.http.bind_address, network.http.bind_port)
                .parse::<SocketAddr>()
                .unwrap_or_else(|_| panic!("Invalid bind address or port for network {}", network_name));

            tracing::info!(
                "🚀 Starting HTTP server for network '{}' on '{}'",
                network_name,
                addr
            );

            let listener = TcpListener::bind(addr)
                .await
                .unwrap_or_else(|err| panic!("Failed to bind to address {addr}: {err}"));

            if let Err(err) = serve(listener, base_app).await {
                tracing::error!(
                    "Server for network '{}' encountered an error: {}",
                    network_name,
                    err
                );
            }
        });
    }

    // Block on ctrl-c
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl-c signal");
}
