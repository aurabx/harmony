pub mod config;
pub mod endpoints;
pub mod backends;
pub mod middleware;
pub mod network;
pub mod groups;
mod router;

use std::net::SocketAddr;
use crate::config::Config;
use tracing_subscriber::{self, prelude::*}; // Add prelude import

pub async fn run(config: Config) {
    // Initialize logging
    if config.logging.log_to_file {
        // Create a file appender
        let file_appender = tracing_subscriber::fmt::layer()
            .with_file(true)
            .with_line_number(true)
            .with_writer(std::fs::File::create(&config.logging.log_file_path).unwrap());

        // Create a stdout appender
        let stdout_appender = tracing_subscriber::fmt::layer()
            .with_file(true)
            .with_line_number(true);

        // Combine both appenders
        tracing_subscriber::registry()
            .with(file_appender)
            .with(stdout_appender)
            .try_init()
            .expect("Failed to initialize logging");
    } else {
        // Just stdout if file logging is disabled
        tracing_subscriber::fmt()
            .with_file(true)
            .with_line_number(true)
            .init();
    }

    tracing::info!("🔧 Starting Harmony '{}'", config.proxy.id);

    // Create a vector to store all server tasks
    let mut server_tasks = Vec::new();

    // Build routers for each network
    for (network_name, network) in &config.network {
        // Build a router specific to this network
        let app = router::build_network_router(&config, network_name).await;

        // Parse the bind address from network config
        let addr: SocketAddr = format!("{}:{}",
                                       network.http.bind_address,
                                       network.http.bind_port
        ).parse().unwrap_or_else(|_| {
            panic!("Invalid bind address or port for network {}", network_name)
        });

        tracing::info!("🚀 Starting HTTP server for network {} on {}", network_name, addr);

        // Create the server task
        let server = axum_server::bind(addr).serve(app.into_make_service());
        server_tasks.push(server);
    }

    // Wait for all servers to complete (they run indefinitely unless there's an error)
    futures::future::join_all(server_tasks).await;
}