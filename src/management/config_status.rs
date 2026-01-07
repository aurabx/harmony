use crate::adapters::registry::AdapterRegistry;
use crate::globals;
use crate::models::mesh::config::{Mesh, MeshEgress, MeshIngress, RemoteIngress};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub bind_address: String,
    pub bind_port: u16,
    pub interface: String,
    pub wireguard_enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigStatusResponse {
    pub config_file_path: String,
    pub config_last_modified: Option<String>,
    pub current_networks: HashMap<String, NetworkStatus>,
    pub running_networks: Vec<String>,
    pub proxy_id: String,
    pub log_level: String,
    /// Mesh configurations
    pub mesh: HashMap<String, Mesh>,
    /// Ingress definitions (extracted from pipelines)
    pub ingress: HashMap<String, MeshIngress>,
    /// Egress definitions (extracted from pipelines)
    pub egress: HashMap<String, MeshEgress>,
    /// Remote ingress definitions (URLs of remote mesh members)
    pub remote_ingress: HashMap<String, RemoteIngress>,
}

/// Handle config status request
pub async fn handle_config_status(
    config_path: String,
    registry: Arc<AdapterRegistry>,
) -> Result<serde_json::Value, (u16, String)> {
    // Get current config
    let config =
        globals::get_config().ok_or_else(|| (500, "No config currently loaded".to_string()))?;

    // Get file modification time
    let config_last_modified = fs::metadata(&config_path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| {
            time.duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .map(|duration| {
                    chrono::DateTime::from_timestamp(duration.as_secs() as i64, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_else(|| "Unknown".to_string())
                })
        });

    // Build network status map
    let mut current_networks = HashMap::new();
    for (network_name, network_config) in &config.network {
        // Use TCP config if present; otherwise, fall back to empty/zero values for status
        let (bind_address, bind_port) = if let Some(tcp) = &network_config.tcp_config {
            (tcp.bind_address.clone(), tcp.bind_port)
        } else {
            ("".to_string(), 0)
        };

        current_networks.insert(
            network_name.clone(),
            NetworkStatus {
                bind_address,
                bind_port,
                interface: network_config.interface.clone(),
                wireguard_enabled: network_config.enable_wireguard,
            },
        );
    }

    // Get running networks from registry
    let running_networks = registry.get_running_networks().await;

    let response = ConfigStatusResponse {
        config_file_path: config_path,
        config_last_modified,
        current_networks,
        running_networks,
        proxy_id: config.proxy.effective_id().to_string(),
        log_level: config.logging.log_level.clone(),
        mesh: config.mesh.clone(),
        ingress: config.ingress.clone(),
        egress: config.egress.clone(),
        remote_ingress: config.remote_ingress.clone(),
    };

    serde_json::to_value(response).map_err(|_| (500, "Failed to serialize response".to_string()))
}
