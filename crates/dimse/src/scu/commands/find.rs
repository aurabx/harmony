//! C-FIND command handler

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

use crate::config::RemoteNode;
use crate::scu::dcmtk_builder::DcmtkCommandBuilder;
use crate::types::{DatasetStream, FindQuery};
use crate::Result;

/// Handle C-FIND request using DCMTK findscu
#[cfg(feature = "dcmtk_cli")]
pub async fn handle_find(
    builder: &DcmtkCommandBuilder,
    node: &RemoteNode,
    query: FindQuery,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    use tokio::process::Command;
    use uuid::Uuid;

    info!(
        "Sending C-FIND to {}@{}:{} (level: {}, max_results: {})",
        node.ae_title, node.host, node.port, query.query_level, query.max_results
    );

    node.validate()?;
    debug!("C-FIND query parameters: {:?}", query.parameters);

    let mut args = builder.build_find_args(node, &query);

    // Output directory for matches under storage_dir/dcmtk
    let dcmtk_base = builder.storage_dir().join("dcmtk");
    let out_dir = dcmtk_base.join(format!("find_{}", Uuid::new_v4()));
    if let Err(e) = tokio::fs::create_dir_all(&out_dir).await {
        warn!("Failed to create output dir {:?}: {}", out_dir, e);
    } else {
        // DCMTK findscu options to write matches to directory
        args.push("-X".into()); // extract responses to DICOM files
        args.push("-od".into());
        args.push(out_dir.to_string_lossy().to_string());
    }

    // Prepare channel to stream results
    let (tx, rx) = mpsc::channel(100);

    debug!("Running findscu args: {:?}", args);
    let tx_clone = tx.clone();
    let out_dir_clone = out_dir.clone();
    tokio::spawn(async move {
        let cleanup_dir;
        match Command::new("findscu").args(&args).output().await {
            Ok(out) => {
                if out.status.success() {
                    info!("C-FIND completed (findscu success)");
                    // Read produced files and convert to in-memory streams immediately
                    if let Ok(mut rd) = tokio::fs::read_dir(&out_dir_clone).await {
                        while let Ok(Some(entry)) = rd.next_entry().await {
                            let path = entry.path();
                            if path.extension().and_then(|s| s.to_str()).unwrap_or("") == "dcm"
                            {
                                // Read file contents immediately to avoid race condition with cleanup
                                if let Ok(bytes) = tokio::fs::read(&path).await {
                                    use bytes::Bytes;
                                    let _ = tx_clone
                                        .send(Ok(DatasetStream::from_bytes(Bytes::from(bytes))))
                                        .await;
                                } else {
                                    warn!("Failed to read C-FIND result file: {:?}", path);
                                }
                            }
                        }
                    }
                    cleanup_dir = out_dir_clone.clone();
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    warn!(
                        "findscu failed: status={:?}, stdout={}, stderr={}",
                        out.status.code(),
                        stdout,
                        stderr
                    );
                    cleanup_dir = out_dir_clone.clone();
                }
            }
            Err(e) => {
                warn!("Failed to spawn findscu: {}", e);
                cleanup_dir = out_dir_clone.clone();
            }
        }

        // Clean up the temporary directory
        if let Err(e) = tokio::fs::remove_dir_all(&cleanup_dir).await {
            warn!(
                "Failed to cleanup C-FIND temp directory {:?}: {}",
                cleanup_dir, e
            );
        } else {
            debug!("🧹 Cleaned up C-FIND temp directory: {:?}", cleanup_dir);
        }

        // drop sender to close stream
    });

    let stream = ReceiverStream::new(rx);
    Ok(stream)
}

#[cfg(not(feature = "dcmtk_cli"))]
pub async fn handle_find(
    _builder: &DcmtkCommandBuilder,
    _node: &RemoteNode,
    _query: FindQuery,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    // No CLI available; return empty stream
    let (_tx, rx) = mpsc::channel(0);
    let stream = ReceiverStream::new(rx);
    Ok(stream)
}
