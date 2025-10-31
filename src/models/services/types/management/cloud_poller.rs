use crate::adapters::registry::AdapterRegistry;
use crate::config::config::Config;
use crate::config::Cli;
use crate::config::reload::compute_diff;
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

/// Poll for config changes and apply them
async fn poll_and_apply_changes(
    client: &RunbeamClient,
    gateway_token: &str,
    registry: &Arc<AdapterRegistry>,
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

    // Process each change
    for change in changes {
        tracing::info!(
            "Processing config change: id={}, summary={}",
            change.id,
            change.summary
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

        // Apply the change
        match apply_cloud_config(&detail.id, &detail.content, registry).await {
            Ok(()) => {
                tracing::info!("✓ Successfully applied config change {}", detail.id);

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
                tracing::error!("✗ Failed to apply config change {}: {}", detail.id, e);

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

/// Apply a config change from the cloud
async fn apply_cloud_config(
    change_id: &str,
    config_content: &str,
    registry: &Arc<AdapterRegistry>,
) -> Result<(), String> {
    // Write config to temp file
    let temp_path = format!("./tmp/cloud_config_{}.toml", change_id);
    
    // Ensure tmp directory exists
    std::fs::create_dir_all("./tmp")
        .map_err(|e| format!("Failed to create tmp directory: {}", e))?;

    std::fs::write(&temp_path, config_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    tracing::info!("Wrote cloud config to {}", temp_path);

    // Load and validate config
    let new_config = load_and_validate_config(&temp_path)?;

    // Get current config
    let old_config = globals::get_config()
        .ok_or_else(|| "No config currently loaded".to_string())?;

    // Compute diff
    let diff = compute_diff(&old_config, &new_config);

    if !diff.has_changes() {
        tracing::info!("No changes detected in cloud config");
        return Ok(());
    }

    tracing::info!(
        "Cloud config diff: zero-downtime={:?}, adapter-restarts={:?}",
        diff.zero_downtime_changes,
        diff.adapter_restarts_required
    );

    let new_config_arc = Arc::new(new_config);

    // Handle network topology changes (adapter restarts)
    if diff.requires_adapter_restart() {
        // Remove networks
        for network in &diff.networks_to_remove {
            registry
                .stop_network(network)
                .await
                .map_err(|e| format!("Failed to stop network '{}': {}", network, e))?;
        }

        // Restart changed networks
        for network in &diff.adapter_restarts_required {
            registry
                .restart_network(network.clone(), new_config_arc.clone())
                .await
                .map_err(|e| format!("Failed to restart network '{}': {}", network, e))?;
        }

        // Add new networks
        for network in &diff.networks_to_add {
            registry
                .start_network(network.clone(), new_config_arc.clone())
                .await
                .map_err(|e| format!("Failed to start network '{}': {}", network, e))?;
        }
    }

    // Update global config (zero-downtime swap)
    globals::set_config(new_config_arc);

    // Update config path to point to the cloud config
    globals::set_config_path(temp_path);

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

/// Load and validate config, returning validation errors if any
fn load_and_validate_config(config_path: &str) -> Result<Config, String> {
    let cli = Cli::new(config_path.to_string());

    // Catch panics from Config::from_args validation
    let result = std::panic::catch_unwind(|| Config::from_args(cli));

    match result {
        Ok(config) => Ok(config),
        Err(err) => {
            // Extract error message from panic
            let error_msg = if let Some(s) = err.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = err.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "Config validation failed with unknown error".to_string()
            };
            Err(error_msg)
        }
    }
}
