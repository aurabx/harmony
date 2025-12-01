//! C-GET command handler

use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

use crate::config::RemoteNode;
use crate::scu::dcmtk_builder::DcmtkCommandBuilder;
use crate::types::{DatasetStream, GetQuery};
use crate::Result;

/// Handle C-GET request using DCMTK getscu
#[cfg(feature = "dcmtk_cli")]
pub async fn handle_get(
    builder: &DcmtkCommandBuilder,
    node: &RemoteNode,
    query: GetQuery,
    output_dir: Option<PathBuf>,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    use tokio::process::Command;
    use uuid::Uuid;

    info!(
        "Sending C-GET to {}@{}:{} (level: {})",
        node.ae_title, node.host, node.port, query.query_level,
    );

    node.validate()?;
    debug!("C-GET query parameters: {:?}", query.parameters);

    let mut args = builder.build_get_args(node, &query);

    // Output directory for received objects
    let (out_dir, should_cleanup) = if let Some(dir) = output_dir {
        // Use caller-provided directory - don't clean up after use
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!("Failed to ensure output dir {:?}: {}", dir, e);
        }
        (dir, false)
    } else {
        // Create our own temporary directory - clean up after use
        let dcmtk_base = builder.storage_dir().join("dcmtk");
        let tmp = dcmtk_base.join(format!("get_{}", Uuid::new_v4()));
        if let Err(e) = tokio::fs::create_dir_all(&tmp).await {
            warn!("Failed to create get output dir {:?}: {}", tmp, e);
        }
        (tmp, true)
    };
    args.push("-od".into());
    args.push(out_dir.to_string_lossy().to_string());

    // Prepare streaming channel
    let (tx, rx) = mpsc::channel(100);

    debug!("Running getscu args: {:?}", args);
    let tx_clone = tx.clone();
    let out_dir_clone = out_dir.clone();
    tokio::spawn(async move {
        let cleanup_dir;
        match Command::new("getscu").args(&args).output().await {
            Ok(out) => {
                if out.status.success() {
                    info!("C-GET completed (getscu success)");
                    // Enumerate received files and stream them back
                    if let Ok(mut rd) = tokio::fs::read_dir(&out_dir_clone).await {
                        while let Ok(Some(entry)) = rd.next_entry().await {
                            let path = entry.path();
                            if let Ok(meta) = tokio::fs::metadata(&path).await {
                                if meta.is_file() {
                                    // Only auto-cleanup files when using our own temp directory
                                    let _ = tx_clone
                                        .send(Ok(DatasetStream::from_file(
                                            path,
                                            should_cleanup,
                                        )))
                                        .await;
                                }
                            }
                        }
                    }
                    cleanup_dir = out_dir_clone.clone();
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    warn!(
                        "getscu failed: status={:?}, stdout={}, stderr={}",
                        out.status.code(),
                        stdout,
                        stderr
                    );
                    cleanup_dir = out_dir_clone.clone();
                }
            }
            Err(e) => {
                warn!("Failed to spawn getscu: {}", e);
                cleanup_dir = out_dir_clone.clone();
            }
        }

        // Only clean up directories that we created ourselves
        if should_cleanup {
            if let Err(e) = tokio::fs::remove_dir_all(&cleanup_dir).await {
                warn!(
                    "Failed to cleanup C-GET temp directory {:?}: {}",
                    cleanup_dir, e
                );
            } else {
                debug!("🧹 Cleaned up C-GET temp directory: {:?}", cleanup_dir);
            }
        }

        // drop sender to close stream
    });

    let stream = ReceiverStream::new(rx);
    Ok(stream)
}

#[cfg(not(feature = "dcmtk_cli"))]
pub async fn handle_get(
    _builder: &DcmtkCommandBuilder,
    _node: &RemoteNode,
    _query: GetQuery,
    _output_dir: Option<PathBuf>,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    // No CLI available; return empty stream
    let (_tx, rx) = mpsc::channel(0);
    let stream = ReceiverStream::new(rx);
    Ok(stream)
}
