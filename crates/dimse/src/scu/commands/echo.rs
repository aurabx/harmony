//! C-ECHO command handler

use tracing::{debug, error, info};

use crate::config::RemoteNode;
use crate::scu::dcmtk_builder::DcmtkCommandBuilder;
use crate::{DimseError, Result};

/// Handle C-ECHO request using DCMTK echoscu
#[cfg(feature = "dcmtk_cli")]
pub async fn handle_echo(
    builder: &DcmtkCommandBuilder,
    node: &RemoteNode,
) -> Result<bool> {
    use tokio::process::Command;

    info!(
        "Sending C-ECHO to {}@{}:{}",
        node.ae_title, node.host, node.port
    );

    // Validate the remote node configuration
    node.validate()?;

    let args = builder.build_echo_args(node);
    debug!(
        "Running: echoscu {}",
        args.join(" ")
    );

    let output = Command::new("echoscu")
        .args(&args)
        .output()
        .await
        .map_err(|e| {
            DimseError::operation_failed(format!("Failed to spawn echoscu: {}", e))
        })?;

    if output.status.success() {
        info!("C-ECHO completed successfully");
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        error!(
            "C-ECHO failed: status={:?}, stdout={}, stderr={}",
            output.status.code(),
            stdout,
            stderr
        );
        Err(DimseError::operation_failed(format!(
            "echoscu failed: {:?} {}",
            output.status.code(),
            stderr
        )))
    }
}

#[cfg(not(feature = "dcmtk_cli"))]
pub async fn handle_echo(
    _builder: &DcmtkCommandBuilder,
    _node: &RemoteNode,
) -> Result<bool> {
    Err(DimseError::NotSupported(
        "C-ECHO requires feature 'dcmtk_cli' or a native UL implementation".into(),
    ))
}
