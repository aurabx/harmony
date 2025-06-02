pub mod config;
pub mod endpoints;
pub mod middleware; // We'll create this next

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

    // Build the router once for all endpoints
    let app = endpoints::build_router(&config).await;
    
    // Parse the bind address from config
    let addr: SocketAddr = format!("{}:{}",
                                   config.network.http.bind_address,
                                   config.network.http.bind_port
    ).parse()
        .expect("Invalid bind address or port");

    tracing::info!("🚀 Starting HTTP server on {}", addr);
    
    // Create and run the server using axum-server
    axum_server::bind(addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}