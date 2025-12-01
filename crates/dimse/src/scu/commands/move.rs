//! C-MOVE command handler

use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

use crate::config::RemoteNode;
use crate::scu::dcmtk_builder::DcmtkCommandBuilder;
use crate::types::{DatasetStream, MoveQuery};
use crate::Result;

/// Handle C-MOVE request using DCMTK movescu
#[cfg(feature = "dcmtk_cli")]
pub async fn handle_move(
    builder: &DcmtkCommandBuilder,
    node: &RemoteNode,
    query: MoveQuery,
    output_dir: Option<PathBuf>,
    external_store_scp: bool,
    incoming_store_port: u16,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    use tokio::process::Command;
    use uuid::Uuid;

    info!(
        "Sending C-MOVE to {}@{}:{} (level: {}, dest: {})",
        node.ae_title, node.host, node.port, query.query_level, query.destination_aet
    );

    node.validate()?;
    debug!("C-MOVE query parameters: {:?}", query.parameters);

    let mut args = builder.build_move_args(node, &query, external_store_scp, incoming_store_port);

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
            let dcmtk_base = builder.storage_dir().join("dcmtk");
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
    let args_for_debug = args.clone();
    let storage_dir = builder.storage_dir().clone();
    tokio::spawn(async move {
        let mut cleanup_dir: Option<PathBuf> = None;
        match Command::new("movescu").args(&args).output().await {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                // Write a debug artifact to storage_dir/dcmtk for test introspection
                let debug_payload = serde_json::json!({
                    "args": args_for_debug,
                    "stdout": stdout,
                    "stderr": stderr,
                    "status_code": out.status.code()
                });
                let dcmtk_base = storage_dir.join("dcmtk");
                if let Err(e) = tokio::fs::create_dir_all(&dcmtk_base).await {
                    warn!("Failed to ensure dcmtk base dir exists: {}", e);
                } else if let Err(e) = tokio::fs::write(
                    dcmtk_base.join("movescu_last.json"),
                    debug_payload.to_string(),
                )
                .await
                {
                    warn!("Failed to write movescu_last.json: {}", e);
                }

                if out.status.success() {
                    info!("C-MOVE completed (movescu success)");
                    // Enumerate received files only when we used a transient out_dir
                    if let Some(ref dir) = out_dir_clone {
                        if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
                            while let Ok(Some(entry)) = rd.next_entry().await {
                                let path = entry.path();
                                if let Ok(meta) = tokio::fs::metadata(&path).await {
                                    if meta.is_file() {
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
                    debug!("🧹 Cleaned up C-MOVE temp directory: {:?}", dir);
                }
            }
        }

        // drop sender to close stream
    });

    let stream = ReceiverStream::new(rx);
    Ok(stream)
}

#[cfg(not(feature = "dcmtk_cli"))]
pub async fn handle_move(
    _builder: &DcmtkCommandBuilder,
    _node: &RemoteNode,
    _query: MoveQuery,
    _output_dir: Option<PathBuf>,
    _external_store_scp: bool,
    _incoming_store_port: u16,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    // No CLI available; return empty stream
    let (_tx, rx) = mpsc::channel(0);
    let stream = ReceiverStream::new(rx);
    Ok(stream)
}
