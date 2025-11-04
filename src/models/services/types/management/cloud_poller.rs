use crate::adapters::registry::AdapterRegistry;
use crate::globals;
use runbeam_sdk::{MachineToken, RunbeamClient, load_token};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

/// Start cloud config polling background task
///
/// This task continuously polls Runbeam Cloud for pending config changes
/// and applies them automatically.
pub async fn start_cloud_polling(
    client: RunbeamClient,
    gateway_token: String,
    poll_interval: Duration,
    registry: Arc<AdapterRegistry>,
    shutdown: CancellationToken,
) {
    tracing::info!(
        "🌥️  Starting cloud config polling (interval: {:?})",
        poll_interval
    );

    let mut consecutive_errors = 0u32;
    let max_backoff = Duration::from_secs(300); // 5 minutes

    loop {
        // Check token expiry before each poll iteration
        if let Err(e) = check_token_validity(&gateway_token).await {
            tracing::warn!("Token validation failed: {}. Stopping cloud polling.", e);
            break;
        }

        tokio::select! {
            _ = shutdown.cancelled() => {
                tracing::info!("Cloud config polling stopped");
                break;
            }
            _ = sleep(poll_interval) => {
                match poll_and_apply_changes(&client, &gateway_token, &registry).await {
                    Ok(()) => {
                        // Reset error counter on success
                        if consecutive_errors > 0 {
                            tracing::info!("Cloud polling recovered after {} errors", consecutive_errors);
                            consecutive_errors = 0;
                        }
                    }
                    Err(e) => {
                        // Check if it's an authorization error
                        if e.contains("401") || e.contains("403") || e.contains("Unauthorized") || e.contains("Forbidden") {
                            tracing::error!("Authorization failed: {}. Stopping cloud polling.", e);
                            break;
                        }

                        consecutive_errors += 1;
                        tracing::error!(
                            "Cloud polling error (attempt {}): {}",
                            consecutive_errors,
                            e
                        );

                        // Exponential backoff on consecutive errors
                        if consecutive_errors > 1 {
                            let backoff = Duration::from_secs(2u64.pow(consecutive_errors.min(8)));
                            let backoff = backoff.min(max_backoff);
                            tracing::warn!("Backing off for {:?} before next poll", backoff);
                            
                            tokio::select! {
                                _ = shutdown.cancelled() => break,
                                _ = sleep(backoff) => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // Clear the global polling token when stopping
    crate::globals::stop_cloud_polling();
}

/// Poll for config changes and write them to disk
async fn poll_and_apply_changes(
    client: &RunbeamClient,
    gateway_token: &str,
    _registry: &Arc<AdapterRegistry>,  // Kept for API compatibility but unused
) -> Result<(), String> {
    // List pending changes
    let changes = client
        .list_config_changes(gateway_token)
        .await
        .map_err(|e| format!("Failed to list config changes: {}", e))?;

    if changes.is_empty() {
        tracing::debug!("No pending config changes");
        return Ok(());
    }

    tracing::info!("Found {} pending config change(s)", changes.len());

    // Process each change in reverse order (oldest first)
    for change in changes.into_iter().rev() {
        tracing::info!(
            "Processing change: id={}, type={}, status={}, gateway_id={}, created_at={}",
            change.id,
            change.change_type,
            change.status,
            change.gateway_id,
            change.created_at
        );

        // Get detailed change content
        let detail = match client
            .get_config_change(gateway_token, &change.id)
            .await
        {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(
                    "Failed to get config change {}: {}",
                    change.id,
                    e
                );
                continue;
            }
        };

        // Acknowledge receipt
        if let Err(e) = client
            .acknowledge_config_change(gateway_token, &change.id)
            .await
        {
            tracing::warn!(
                "Failed to acknowledge config change {}: {}",
                change.id,
                e
            );
            // Continue anyway - we still want to try applying
        }

        // Write config file (file watcher will detect and apply)
        match write_cloud_config(&detail.id, &detail.toml_config).await {
            Ok(()) => {
                tracing::info!("✓ Wrote config change {} (file watcher will apply)", detail.id);

                // Note: We report success immediately after writing the file.
                // The file watcher will detect and apply the change asynchronously.
                // If the file watcher fails to apply, it will be logged but not reported back to cloud.
                // This is acceptable because:
                // 1. Config validation happens in the file watcher
                // 2. If invalid, old config remains active (safe)
                // 3. Admin can see file watcher errors in logs
                
                // Report success to cloud
                if let Err(e) = client
                    .report_config_applied(gateway_token, &detail.id)
                    .await
                {
                    tracing::warn!(
                        "Failed to report success for {}: {}",
                        detail.id,
                        e
                    );
                }
            }
            Err(e) => {
                tracing::error!("✗ Failed to write config change {}: {}", detail.id, e);

                // Report failure to cloud
                if let Err(report_err) = client
                    .report_config_failed(gateway_token, &detail.id, &e)
                    .await
                {
                    tracing::warn!(
                        "Failed to report error for {}: {}",
                        detail.id,
                        report_err
                    );
                }
            }
        }
    }

    Ok(())
}

/// Write a config change from the cloud to disk
/// 
/// The file watcher will detect the change and apply it automatically.
/// This separation of concerns ensures:
/// - Single source of truth for config reloading (file watcher)
/// - No race conditions between cloud poller and file watcher
/// - Consistent behavior for both manual edits and cloud changes
async fn write_cloud_config(
    change_id: &str,
    config_content: &str,
) -> Result<(), String> {
    // Get the current config path (where file watcher is watching)
    let target_path = globals::get_config_path()
        .unwrap_or_else(|| "./config/config.toml".to_string());
    
    tracing::info!("Writing cloud config to {}", target_path);
    
    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(&target_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }

    // Write config file (file watcher will detect and apply)
    std::fs::write(&target_path, config_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    tracing::info!("✓ Config written to disk, file watcher will detect and apply changes");
    
    // Also save a backup copy for debugging/audit trail
    let backup_dir = "./tmp/cloud_configs";
    std::fs::create_dir_all(backup_dir).ok();
    let backup_path = format!("{}/config_{}.toml", backup_dir, change_id);
    if let Err(e) = std::fs::write(&backup_path, config_content) {
        tracing::warn!("Failed to write backup config to {}: {}", backup_path, e);
    } else {
        tracing::debug!("Backup saved to {}", backup_path);
    }

    Ok(())
}

/// Check if the stored token is still valid
async fn check_token_validity(gateway_token: &str) -> Result<(), String> {
    // Get proxy ID for instance isolation
    let proxy_id = globals::get_config()
        .map(|config| config.proxy.id.clone())
        .unwrap_or_else(|| "harmony".to_string());

    // Load token from secure storage (SDK manages keyring/encrypted filesystem automatically)
    let token: MachineToken = load_token(&proxy_id, "auth")
        .await
        .map_err(|e| format!("Failed to load token: {}", e))?
        .ok_or_else(|| "No token found in storage".to_string())?;

    // Verify the token matches the one we're using
    if token.machine_token != gateway_token {
        return Err("Token mismatch - token may have been updated".to_string());
    }

    // Check if token has expired
    if token.is_expired() {
        return Err(format!(
            "Token expired at {}. Please re-authorize the gateway.",
            token.expires_at
        ));
    }

    Ok(())
}

