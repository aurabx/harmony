//! C-GET command handler
//!
//! Uses DCMTK getscu because dicom-ul 0.9.0 does not support
//! SCP/SCU Role Selection negotiation required for C-GET operations.

use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

use crate::config::{DimseConfig, RemoteNode};
use crate::types::{DatasetStream, GetQuery};
use crate::Result;

/// Handle C-GET request using DCMTK getscu
pub async fn handle_get(
    config: &DimseConfig,
    node: &RemoteNode,
    query: GetQuery,
    output_dir: Option<PathBuf>,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    use tokio::process::Command;
    use uuid::Uuid;
    use crate::common::query_utils;

    info!(
        "Sending C-GET to {}@{}:{} (level: {})",
        node.ae_title, node.host, node.port, query.query_level,
    );

    node.validate()?;
    debug!("C-GET query parameters: {:?}", query.parameters);

    // Build arguments for getscu (matching old dcmtk_builder implementation)
    let mut args = Vec::new();

    // Use Patient Root or Study Root as per query level
    match query.query_level {
        crate::types::QueryLevel::Patient => args.push("-P".into()),
        crate::types::QueryLevel::Study | crate::types::QueryLevel::Series | crate::types::QueryLevel::Image => {
            args.push("-S".into())
        }
    }

    // Base arguments (AET)
    args.push("-aet".into());
    args.push(config.local_aet.clone());
    args.push("-aec".into());
    args.push(node.ae_title.clone());

    // Add query level parameter
    let level_str = query_utils::query_level_to_string(query.query_level);
    args.push("-k".into());
    args.push(format!("QueryRetrieveLevel={}", level_str));

    // Add query parameters
    for (k, v) in &query.parameters {
        let tag = query_utils::normalize_tag(k);
        args.push("-k".into());
        if v.is_empty() {
            args.push(format!("{}=", tag));
        } else {
            args.push(format!("{}={}", tag, v));
        }
    }

    // Host and port
    args.push(node.host.clone());
    args.push(node.port.to_string());

    // Output directory for received objects
    let (out_dir, should_cleanup) = if let Some(dir) = output_dir {
        // Use caller-provided directory - don't clean up after use
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!("Failed to ensure output dir {:?}: {}", dir, e);
        }
        (dir, false)
    } else {
        // Create our own temporary directory - clean up after use
        let dcmtk_base = PathBuf::from(&config.storage_dir).join("dcmtk");
        let tmp = dcmtk_base.join(format!("get_{}", Uuid::new_v4()));
        if let Err(e) = tokio::fs::create_dir_all(&tmp).await {
            warn!("Failed to create get output dir {:?}: {}", tmp, e);
        }
        (tmp, true)
    };
    
    // Add -od at the very end (critical for getscu to work properly)
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
                
                if !out.stderr.is_empty() {
                    let _stderr = String::from_utf8_lossy(&out.stderr);
                }
                
                if out.status.success() {
                    info!("C-GET completed (getscu success)");
                    // Enumerate received files and stream them back
                    if let Ok(mut rd) = tokio::fs::read_dir(&out_dir_clone).await {
                        let mut count = 0;
                        while let Ok(Some(entry)) = rd.next_entry().await {
                            let path = entry.path();
                            if let Ok(meta) = tokio::fs::metadata(&path).await {
                                if meta.is_file() {
                                    count += 1;
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
                        if count == 0 {
                            warn!("C-GET completed but no files were found in output directory: {}", out_dir_clone.display());
                        }
                    } else {
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
                debug!("Cleaned up C-GET temp directory: {:?}", cleanup_dir);
            }
        }

        // drop sender to close stream
    });

    let stream = ReceiverStream::new(rx);
    Ok(stream)
}
