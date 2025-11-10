use crate::adapters::registry::AdapterRegistry;
use crate::globals;
use runbeam_sdk::{MachineToken, RunbeamClient, load_token};
use std::collections::HashSet;
use std::path::Path;
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
                        // Check if it's an authorization error or missing token
                        if e.contains("401") || e.contains("403") || e.contains("Unauthorized") || e.contains("Forbidden") {
                            tracing::error!("Authorization failed: {}. Stopping cloud polling.", e);
                            break;
                        }
                        
                        // Check if machine token is missing (gateway not authorized)
                        if e.contains("No machine token found") || e.contains("Failed to load machine token") {
                            tracing::warn!("Machine token not found. Gateway needs to be authorized. Stopping cloud polling.");
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
    // Extract gateway_id from the stored machine token
    // The gateway_id is a ULID that was received during authorization
    let proxy_id = globals::get_config()
        .map(|config| config.proxy.id.clone())
        .unwrap_or_else(|| "harmony".to_string());

    let machine_token: MachineToken = load_token(&proxy_id, "auth")
        .await
        .map_err(|e| format!("Failed to load machine token: {}", e))?
        .ok_or_else(|| "No machine token found. Gateway may not be authorized.".to_string())?;

    let gateway_id = machine_token.gateway_id;

    // List pending changes
    let response = client
        .list_changes_for_gateway(gateway_token, &gateway_id)
        .await
        .map_err(|e| format!("Failed to list config changes: {}", e))?;

    let changes = response.data;

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
            change.resource_type,
            change.status.as_deref().unwrap_or("unknown"),
            change.gateway_id,
            change.created_at
        );

        // Get detailed change content
        let detail = match client
            .get_change(gateway_token, &change.id)
            .await
        {
            Ok(response) => response.data,
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
            .acknowledge_changes(gateway_token, vec![change.id.clone()])
            .await
        {
            tracing::warn!(
                "Failed to acknowledge config change {}: {}",
                change.id,
                e
            );
            // Continue anyway - we still want to try applying
        }

        // Determine transforms directory path
        let config_path = globals::get_config_path()
            .unwrap_or_else(|| "./config/config.toml".to_string());
        let config_dir = std::path::Path::new(&config_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        
        let transforms_path = globals::get_config()
            .map(|c| c.proxy.transforms_path.clone())
            .unwrap_or_else(|| "transforms".to_string());
        
        let transforms_dir = config_dir.join(transforms_path);

        // Get the TOML config content - should always be present in detail view
        let toml_config = detail.toml_config.as_ref().ok_or_else(|| {
            format!("Config change {} is missing toml_config field", detail.id)
        })?;

        // Extract and fetch transforms before writing config
        match extract_transform_ids(toml_config) {
            Ok(transform_ids) if !transform_ids.is_empty() => {
                tracing::info!(
                    "Config change {} requires {} transform(s)",
                    detail.id,
                    transform_ids.len()
                );
                
                // Fetch and write transforms
                if let Err(e) = fetch_and_write_transforms(
                    client,
                    gateway_token,
                    transform_ids,
                    &transforms_dir,
                )
                .await
                {
                    tracing::error!(
                        "✗ Failed to download transforms for config change {}: {}",
                        detail.id,
                        e
                    );
                    
                    // Report failure to cloud and skip config write
                    if let Err(report_err) = client
                        .mark_change_failed(
                            gateway_token,
                            &detail.id,
                            format!("Transform download failed: {}", e),
                            None,
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to report transform error for {}: {}",
                            detail.id,
                            report_err
                        );
                    }
                    
                    // Skip to next change
                    continue;
                }
            }
            Ok(_) => {
                tracing::debug!("No transforms required for config change {}", detail.id);
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to extract transform IDs from config {}: {}. Continuing anyway.",
                    detail.id,
                    e
                );
                // Continue - config might still be valid without transforms
            }
        }

        // Write config file (file watcher will detect and apply)
        match write_cloud_config(&detail.id, toml_config).await {
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
                    .mark_change_applied(gateway_token, &detail.id)
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
                    .mark_change_failed(gateway_token, &detail.id, e.clone(), None)
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

/// Extract transform IDs from TOML configuration
///
/// Parses the TOML configuration string and extracts all transform IDs referenced
/// in middleware sections. Returns a unique list of transform IDs that need to be
/// downloaded from Runbeam Cloud.
///
/// # Arguments
///
/// * `toml_config` - The TOML configuration string
///
/// # Returns
///
/// * `Ok(Vec<String>)` - List of unique transform IDs (without .json extension)
/// * `Err(String)` - Error message if TOML parsing fails critically
fn extract_transform_ids(toml_config: &str) -> Result<Vec<String>, String> {
    // Parse TOML configuration
    let config: toml::Value = toml::from_str(toml_config)
        .map_err(|e| format!("Failed to parse TOML config: {}", e))?;

    let mut transform_ids = HashSet::new();

    // Navigate to middleware section
    if let Some(middleware) = config.get("middleware").and_then(|v| v.as_table()) {
        // Iterate through each middleware entry
        for (_middleware_name, middleware_config) in middleware {
            // Check if this is a transform middleware
            if let Some(middleware_type) = middleware_config.get("type").and_then(|v| v.as_str()) {
                if middleware_type == "transform" {
                    // Extract spec_path from options
                    if let Some(spec_path) = middleware_config
                        .get("options")
                        .and_then(|opts| opts.get("spec_path"))
                        .and_then(|v| v.as_str())
                    {
                        // Extract transform ID from spec_path
                        // Handle both "id.json" and "path/to/id.json" formats
                        let filename = spec_path
                            .rsplit('/')
                            .next()
                            .unwrap_or(spec_path);
                        
                        // Remove .json extension if present
                        let transform_id = filename.strip_suffix(".json").unwrap_or(filename);
                        
                        if !transform_id.is_empty() {
                            transform_ids.insert(transform_id.to_string());
                            tracing::debug!("Found transform reference: {}", transform_id);
                        }
                    }
                }
            }
        }
    }

    let ids: Vec<String> = transform_ids.into_iter().collect();
    
    if !ids.is_empty() {
        tracing::info!("Extracted {} unique transform ID(s) from config", ids.len());
    } else {
        tracing::debug!("No transform middleware found in config");
    }

    Ok(ids)
}

/// Fetch transforms from Runbeam Cloud and write them to disk
///
/// Downloads JOLT transformation specifications from Runbeam Cloud and writes
/// them as JSON files in the transforms directory. Existing files are overwritten.
///
/// # Arguments
///
/// * `client` - Runbeam API client
/// * `gateway_token` - Machine token for authentication
/// * `transform_ids` - List of transform IDs to fetch
/// * `transforms_dir` - Directory to write transform files
///
/// # Returns
///
/// * `Ok(())` - All transforms fetched and written successfully
/// * `Err(String)` - Error message if any transform fetch or write fails
async fn fetch_and_write_transforms(
    client: &RunbeamClient,
    gateway_token: &str,
    transform_ids: Vec<String>,
    transforms_dir: &Path,
) -> Result<(), String> {
    if transform_ids.is_empty() {
        return Ok(());
    }

    // Create transforms directory if it doesn't exist
    std::fs::create_dir_all(transforms_dir)
        .map_err(|e| format!("Failed to create transforms directory: {}", e))?;

    tracing::info!(
        "Downloading {} transform(s) to {}",
        transform_ids.len(),
        transforms_dir.display()
    );

    // Fetch and write each transform
    for transform_id in transform_ids {
        tracing::debug!("Fetching transform: {}", transform_id);

        // Fetch transform from API
        let transform_response = client
            .get_transform(gateway_token, &transform_id)
            .await
            .map_err(|e| {
                format!("Failed to fetch transform {}: {}", transform_id, e)
            })?;

        // Extract JOLT specification from response
        let jolt_spec = transform_response
            .data
            .options
            .as_ref()
            .and_then(|opts| opts.instructions.as_ref())
            .ok_or_else(|| {
                format!(
                    "Transform {} does not contain instructions field",
                    transform_id
                )
            })?;

        // Write transform to file
        let filename = format!("{}.json", transform_id);
        let file_path = transforms_dir.join(&filename);
        
        std::fs::write(&file_path, jolt_spec)
            .map_err(|e| format!("Failed to write transform file {}: {}", filename, e))?;

        tracing::info!("✓ Downloaded transform: {} -> {}", transform_id, filename);
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

