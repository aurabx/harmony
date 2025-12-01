//! C-STORE command handler

use std::time::Duration;

use tracing::{debug, info};

use crate::config::RemoteNode;
use crate::scu::dcmtk_builder::DcmtkCommandBuilder;
use crate::types::DatasetStream;
use crate::Result;

/// Handle C-STORE request (stub implementation)
pub async fn handle_store(
    _builder: &DcmtkCommandBuilder,
    node: &RemoteNode,
    dataset: DatasetStream,
) -> Result<bool> {
    info!(
        "Sending C-STORE to {}@{}:{}",
        node.ae_title, node.host, node.port
    );

    // Validate the remote node configuration
    node.validate()?;

    debug!("C-STORE dataset: id={}", dataset.metadata().id);

    // TODO: Implement actual DICOM association and C-STORE
    // This is a stub implementation

    // Simulate sending the dataset
    tokio::time::sleep(Duration::from_millis(300)).await;

    info!("C-STORE completed successfully");
    Ok(true)
}
