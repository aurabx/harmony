//! Connection management utilities for SCU

use std::time::Duration;

use tracing::{error, info, warn};

use crate::config::{DimseConfig, RemoteNode};
use crate::scu::commands::echo;
use crate::{DimseError, Result};

/// Test connectivity to a remote node with retry logic
pub async fn test_connection(
    config: &DimseConfig,
    node: &RemoteNode,
    max_retries: u32,
) -> Result<bool> {
    let mut retries = 0;

    while retries <= max_retries {
        if retries > 0 {
            info!("Connection test retry {} of {}", retries, max_retries);
            tokio::time::sleep(Duration::from_secs(1 << retries)).await; // Exponential backoff
        }

        match echo::handle_echo(config, node).await {
            Ok(_) => {
                info!("Connection test successful");
                return Ok(true);
            }
            Err(e) if e.is_recoverable() && retries < max_retries => {
                warn!("Connection test failed (attempt {}): {}", retries + 1, e);
                retries += 1;
                continue;
            }
            Err(e) => {
                error!("Connection test failed permanently: {}", e);
                return Err(e);
            }
        }
    }

    Err(DimseError::operation_failed(
        "Connection test failed after all retries",
    ))
}

/// Get connection timeout for a node (uses node-specific or global setting)
pub fn get_connection_timeout(config: &DimseConfig, node: &RemoteNode) -> Duration {
    node.connect_timeout_ms
        .map(Duration::from_millis)
        .unwrap_or_else(|| config.connect_timeout())
}

/// Get maximum PDU size for a node (uses node-specific or global setting)
pub fn get_max_pdu(config: &DimseConfig, node: &RemoteNode) -> u32 {
    node.max_pdu.unwrap_or(config.max_pdu)
}
