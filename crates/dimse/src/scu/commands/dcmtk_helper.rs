//! Common helpers for DCMTK command execution
//!
//! This module provides shared functionality for running DCMTK CLI tools
//! like getscu and movescu, reducing code duplication.

use std::path::PathBuf;
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::DimseConfig;
use crate::types::DatasetStream;
use crate::Result;

/// Configuration for a DCMTK command execution
pub struct DcmtkCommand {
    /// The command to run (e.g., "getscu", "movescu")
    pub command: String,
    /// Command arguments
    pub args: Vec<String>,
    /// Output directory for received files
    pub output_dir: PathBuf,
    /// Whether to clean up the output directory after use
    pub cleanup_on_complete: bool,
    /// Operation name for logging
    pub operation_name: String,
}

impl DcmtkCommand {
    /// Create a new DCMTK command configuration
    pub fn new(command: impl Into<String>, operation_name: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            output_dir: PathBuf::new(),
            cleanup_on_complete: false,
            operation_name: operation_name.into(),
        }
    }

    /// Add an argument
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add multiple arguments
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(|a| a.into()));
        self
    }

    /// Set the output directory
    pub fn output_dir(mut self, dir: PathBuf, cleanup: bool) -> Self {
        self.output_dir = dir;
        self.cleanup_on_complete = cleanup;
        self
    }
}

/// Prepare output directory for DCMTK commands
/// Returns (output_dir, should_cleanup)
pub async fn prepare_output_dir(
    config: &DimseConfig,
    provided_dir: Option<PathBuf>,
    prefix: &str,
) -> (PathBuf, bool) {
    if let Some(dir) = provided_dir {
        // Use caller-provided directory - don't clean up after use
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!("Failed to ensure output dir {:?}: {}", dir, e);
        }
        (dir, false)
    } else {
        // Create our own temporary directory - clean up after use
        let dcmtk_base = PathBuf::from(&config.storage_dir).join("dcmtk");
        let tmp = dcmtk_base.join(format!("{}_{}", prefix, Uuid::new_v4()));
        if let Err(e) = tokio::fs::create_dir_all(&tmp).await {
            warn!("Failed to create {} output dir {:?}: {}", prefix, tmp, e);
        }
        (tmp, true)
    }
}

/// Execute a DCMTK command and stream results back through a channel
pub fn spawn_dcmtk_command(
    cmd: DcmtkCommand,
    tx: mpsc::Sender<Result<DatasetStream>>,
) {
    let DcmtkCommand {
        command,
        args,
        output_dir,
        cleanup_on_complete,
        operation_name,
    } = cmd;

    tokio::spawn(async move {
        debug!("Running {} with args: {:?}", command, args);

        match Command::new(&command).args(&args).output().await {
            Ok(output) => {
                if output.status.success() {
                    info!("{} completed successfully", operation_name);
                    
                    // Stream files from output directory
                    if let Err(e) = stream_directory_files(&output_dir, &tx, cleanup_on_complete).await {
                        warn!("Error streaming files: {}", e);
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    warn!(
                        "{} failed: status={:?}, stdout={}, stderr={}",
                        command,
                        output.status.code(),
                        stdout.chars().take(200).collect::<String>(),
                        stderr.chars().take(200).collect::<String>()
                    );
                }
            }
            Err(e) => {
                warn!("Failed to spawn {}: {}", command, e);
            }
        }

        // Clean up if needed
        if cleanup_on_complete && output_dir.exists() {
            if let Err(e) = tokio::fs::remove_dir_all(&output_dir).await {
                warn!("Failed to cleanup {} temp directory {:?}: {}", operation_name, output_dir, e);
            } else {
                debug!("Cleaned up {} temp directory: {:?}", operation_name, output_dir);
            }
        }
    });
}

/// Stream all files from a directory through the channel
async fn stream_directory_files(
    dir: &PathBuf,
    tx: &mpsc::Sender<Result<DatasetStream>>,
    delete_on_drop: bool,
) -> std::io::Result<()> {
    let mut rd = tokio::fs::read_dir(dir).await?;
    let mut count = 0;

    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if let Ok(meta) = tokio::fs::metadata(&path).await {
            if meta.is_file() {
                count += 1;
                let _ = tx
                    .send(Ok(DatasetStream::from_file(path, delete_on_drop)))
                    .await;
            }
        }
    }

    if count == 0 {
        warn!("No files found in output directory: {}", dir.display());
    } else {
        debug!("Streamed {} files from {}", count, dir.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dcmtk_command_builder() {
        let cmd = DcmtkCommand::new("getscu", "C-GET")
            .arg("-S")
            .args(["-aet", "TEST_SCU"])
            .output_dir(PathBuf::from("/tmp/test"), true);

        assert_eq!(cmd.command, "getscu");
        assert_eq!(cmd.args, vec!["-S", "-aet", "TEST_SCU"]);
        assert_eq!(cmd.output_dir, PathBuf::from("/tmp/test"));
        assert!(cmd.cleanup_on_complete);
    }
}
