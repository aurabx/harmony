//! C-MOVE command handler
//!
//! Uses DCMTK movescu because it provides reliable Store SCP functionality

use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

use crate::config::{DimseConfig, RemoteNode};
use crate::types::{DatasetStream, MoveQuery};
use crate::Result;

/// Handle C-MOVE request using DCMTK movescu
pub async fn handle_move(
    config: &DimseConfig,
    node: &RemoteNode,
    query: MoveQuery,
    output_dir: Option<PathBuf>,
    external_store_scp: bool,
    incoming_store_port: u16,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    use tokio::process::Command;
    use uuid::Uuid;
    use crate::common::query_utils;

    info!(
        "Sending C-MOVE to {}@{}:{} (level: {}, dest: {})",
        node.ae_title, node.host, node.port, query.query_level, query.destination_aet
    );

    node.validate()?;
    debug!("C-MOVE query parameters: {:?}", query.parameters);

    // Build arguments for movescu (matching old dcmtk_builder implementation)
    let mut args = vec![
        "-d".into(), // Enable verbose output for diagnostics
        "-S".into(), // Use Study Root query model for C-MOVE
    ];
    
    // Base arguments (AET)
    args.push("-aet".into());
    args.push(config.local_aet.clone());
    args.push("-aec".into());
    args.push(node.ae_title.clone());

    // Move destination AET
    args.push("-aem".into());
    args.push(query.destination_aet.clone());

    // Add query level parameter
    let level_str = query_utils::query_level_to_string(query.query_level);
    args.push("-k".into());
    args.push(format!("0008,0052={}", level_str));

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

    // Incoming C-STORE handling
    // If using an external persistent Store SCP, do not open a transient listener (+P)
    if !external_store_scp {
        args.push("+P".into());
        args.push(incoming_store_port.to_string());
    }

    // Host and port at the end
    args.push(node.host.clone());
    args.push(node.port.to_string());

    // Output directory for received objects (only when using transient +P listener)
    let mut out_dir_opt: Option<PathBuf> = None;
    let mut should_cleanup_move = false;
    if !external_store_scp {
        let out_dir = if let Some(dir) = output_dir {
            // Use caller-provided directory - don't clean up after use
            if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                warn!("Failed to ensure output dir {:?}: {}", dir, e);
            }
            dir
        } else {
            // Create our own temporary directory - clean up after use
            let dcmtk_base = PathBuf::from(&config.storage_dir).join("dcmtk");
            let tmp = dcmtk_base.join(format!("move_{}", Uuid::new_v4()));
            if let Err(e) = tokio::fs::create_dir_all(&tmp).await {
                warn!("Failed to create move output dir {:?}: {}", tmp, e);
            }
            should_cleanup_move = true;
            tmp
        };
        args.push("-od".into());
        args.push(out_dir.to_string_lossy().to_string());
        out_dir_opt = Some(out_dir);
    }

    // Prepare streaming channel
    let (tx, rx) = mpsc::channel(100);

    info!("Running movescu with args: {:?}", args);
    
    let tx_clone = tx.clone();
    let out_dir_clone = out_dir_opt.clone();
    let _storage_dir = PathBuf::from(&config.storage_dir);
    tokio::spawn(async move {
        let mut cleanup_dir: Option<PathBuf> = None;
        match Command::new("movescu").args(&args).output().await {
            Ok(out) => {
                
                if !out.stderr.is_empty() {
                    let _stderr = String::from_utf8_lossy(&out.stderr);
                }
                
                if out.status.success() {
                    info!("C-MOVE completed (movescu success)");
                    // Enumerate received files only when we used a transient out_dir
                    if let Some(ref dir) = out_dir_clone {
                        if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
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
                                                should_cleanup_move,
                                            )))
                                            .await;
                                    }
                                }
                            }
                            if count == 0 {
                                warn!("C-MOVE completed but no files were found in output directory: {}", dir.display());
                            }
                        }
                        cleanup_dir = Some(dir.clone());
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    warn!(
                        "movescu failed: status={:?}, stdout=\n{}\nstderr=\n{}",
                        out.status.code(),
                        stdout,
                        stderr
                    );
                    cleanup_dir = out_dir_clone.clone();
                }
            }
            Err(e) => {
                warn!("Failed to spawn movescu: {}", e);
                cleanup_dir = out_dir_clone.clone();
            }
        }

        // Only clean up directories that we created ourselves
        if should_cleanup_move {
            if let Some(dir) = cleanup_dir {
                if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
                    warn!("Failed to cleanup C-MOVE temp directory {:?}: {}", dir, e);
                } else {
                    debug!("Cleaned up C-MOVE temp directory: {:?}", dir);
                }
            }
        }

        // drop sender to close stream
    });

    let stream = ReceiverStream::new(rx);
    Ok(stream)
}
