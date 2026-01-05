use runbeam_sdk::{load_token, RunbeamClient};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Request body for update (empty POST)
#[derive(Debug, Deserialize)]
pub struct UpdateRequest {}

/// Response for successful configuration update
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateResponse {
    /// Success status
    pub success: bool,
    /// Message describing the result
    pub message: String,
    /// Size of uploaded configuration in bytes
    pub config_size: usize,
    /// Number of pipeline configs uploaded
    #[serde(default)]
    pub pipeline_count: usize,
    /// Number of transform configs uploaded
    #[serde(default)]
    pub transform_count: usize,
    /// Number of mesh configs uploaded
    #[serde(default)]
    pub mesh_count: usize,
}

/// Handle POST request to upload current configuration to Runbeam Cloud
///
/// This endpoint:
/// 1. Loads the machine token from secure storage
/// 2. Reads the current configuration TOML file
/// 3. Parses it to extract the gateway ID
/// 4. Calls Runbeam Cloud API to store the configuration
/// 5. Returns success/failure response
pub async fn handle_update() -> Result<(Value, u16), (u16, String)> {
    tracing::info!("Processing configuration update request");

    // Get proxy ID for instance isolation
    let proxy_id = crate::globals::get_config()
        .map(|config| config.proxy.effective_id().to_string())
        .unwrap_or_else(|| "harmony".to_string());

    // Load machine token from secure storage
    tracing::debug!("Loading machine token for proxy: {}", proxy_id);
    let machine_token: runbeam_sdk::MachineToken = load_token(&proxy_id, "auth")
        .await
        .map_err(|e| {
            tracing::error!("Failed to load machine token: {}", e);
            (
                401,
                "Not authorized. Run `runbeam harmony:authorize` first to obtain a machine token."
                    .to_string(),
            )
        })?
        .ok_or_else(|| {
            tracing::error!("No machine token found for proxy: {}", proxy_id);
            (
                401,
                "Not authorized. Run `runbeam harmony:authorize` first to obtain a machine token."
                    .to_string(),
            )
        })?;

    tracing::debug!("Machine token loaded successfully");

    // Get config file path from globals
    let config_path = crate::globals::get_config_path().ok_or_else(|| {
        tracing::error!("Configuration path not available");
        (500, "Configuration file path not accessible".to_string())
    })?;

    tracing::info!("Reading configuration from: {}", config_path);

    // Read main TOML config file (gateway config only)
    let gateway_config = std::fs::read_to_string(&config_path).map_err(|e| {
        tracing::error!("Failed to read configuration file: {}", e);
        (500, format!("Failed to read configuration file: {}", e))
    })?;

    let config_dir = Path::new(&config_path).parent().unwrap_or(Path::new("."));
    
    // Collect pipeline configs from the pipelines/ directory
    let pipelines_dir = config_dir.join("pipelines");
    let mut pipeline_configs: Vec<(String, String)> = Vec::new();
    
    if pipelines_dir.exists() && pipelines_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&pipelines_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "toml") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let filename = path.file_name()
                            .unwrap_or_default()
                            .to_str()
                            .unwrap_or("")
                            .to_string();
                        tracing::debug!("Found pipeline config: {:?}", path);
                        pipeline_configs.push((filename, content));
                    }
                }
            }
        }
    }

    // Collect transform configs from the transforms/ directory
    // Note: transforms_path in config determines the directory name
    let transforms_path = crate::globals::get_config()
        .map(|cfg| cfg.proxy.transforms_path.clone())
        .unwrap_or_else(|| "transforms".to_string());
    let transforms_dir = config_dir.join(&transforms_path);
    let mut transform_configs: Vec<(String, String)> = Vec::new();
    
    if transforms_dir.exists() && transforms_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&transforms_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                // Transforms can be .json (JOLT specs) - we wrap them in TOML format
                if path.extension().map_or(false, |ext| ext == "json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let filename = path.file_name()
                            .unwrap_or_default()
                            .to_str()
                            .unwrap_or("")
                            .to_string();
                        // Extract transform name from filename (without extension)
                        let name = path.file_stem()
                            .unwrap_or_default()
                            .to_str()
                            .unwrap_or("")
                            .to_string();
                        // Wrap JSON spec in TOML format for the API
                        let toml_content = format!(
                            "[transform.{}]\nname = \"{}\"\ninstructions = '''\n{}\n'''",
                            name, name, content
                        );
                        tracing::debug!("Found transform config: {:?}", path);
                        transform_configs.push((filename, toml_content));
                    }
                }
            }
        }
    }

    // Collect mesh configs from the mesh/ directory
    let mesh_dir = config_dir.join("mesh");
    let mut mesh_configs: Vec<(String, String)> = Vec::new();
    
    if mesh_dir.exists() && mesh_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&mesh_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "toml") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let filename = path.file_name()
                            .unwrap_or_default()
                            .to_str()
                            .unwrap_or("")
                            .to_string();
                        tracing::debug!("Found mesh config: {:?}", path);
                        mesh_configs.push((filename, content));
                    }
                }
            }
        }
    }

    let config_size = gateway_config.len();
    tracing::debug!("Gateway configuration size: {} bytes", config_size);
    tracing::debug!("Found {} pipeline config(s)", pipeline_configs.len());
    tracing::debug!("Found {} transform config(s)", transform_configs.len());
    tracing::debug!("Found {} mesh config(s)", mesh_configs.len());

    // Use gateway_id from machine token (this is the ULID from Runbeam Cloud)
    let gateway_id = &machine_token.gateway_id;

    tracing::info!("Uploading configuration for gateway: {}", gateway_id);

    // Get API base URL from global config
    let api_base_url = crate::globals::get_config()
        .map(|cfg| cfg.runbeam.effective_cloud_api_base_url())
        .unwrap_or_else(|| "https://api.runbeam.cloud".to_string());

    tracing::debug!("Using Runbeam API base URL: {}", api_base_url);

    // Create Runbeam Cloud API client and discover actual base URL
    let client = RunbeamClient::new(api_base_url);
    let client = match client.discover_base_url(&machine_token.machine_token).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Base URL discovery failed (using configured URL): {}", e);
            client
        }
    };

    // Step 1: Upload gateway configuration
    tracing::info!("Uploading gateway configuration for gateway: {}", gateway_id);
    client
        .store_config(
            &machine_token.machine_token,
            "gateway",
            Some(gateway_id.clone()),
            &gateway_config,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to upload gateway configuration: {}", e);
            map_sdk_error(e)
        })?;
    
    tracing::info!("Gateway configuration uploaded successfully: {} bytes", config_size);

    // Step 2: Upload each pipeline configuration separately
    let mut pipeline_count = 0;
    for (filename, content) in &pipeline_configs {
        tracing::info!("Uploading pipeline configuration from: {}", filename);
        
        match client
            .store_config(
                &machine_token.machine_token,
                "pipeline",
                None, // pipeline ID will be extracted from the TOML content
                content,
            )
            .await
        {
            Ok(response) => {
                tracing::info!(
                    "Pipeline configuration uploaded successfully: {}, action: {}",
                    filename,
                    response.data.action
                );
                pipeline_count += 1;
            }
            Err(e) => {
                tracing::error!("Failed to upload pipeline configuration {}: {}", filename, e);
                // Continue with other configs rather than failing entirely
            }
        }
    }

    // Step 3: Upload each transform configuration separately
    let mut transform_count = 0;
    for (filename, content) in &transform_configs {
        tracing::info!("Uploading transform configuration from: {}", filename);
        
        match client
            .store_config(
                &machine_token.machine_token,
                "transform",
                None, // transform ID will be extracted from the TOML content
                content,
            )
            .await
        {
            Ok(response) => {
                tracing::info!(
                    "Transform configuration uploaded successfully: {}, action: {}",
                    filename,
                    response.data.action
                );
                transform_count += 1;
            }
            Err(e) => {
                tracing::error!("Failed to upload transform configuration {}: {}", filename, e);
                // Continue with other configs rather than failing entirely
            }
        }
    }

    // Step 4: Upload each mesh configuration separately
    let mut mesh_count = 0;
    for (filename, content) in &mesh_configs {
        tracing::info!("Uploading mesh configuration from: {}", filename);
        
        match client
            .store_config(
                &machine_token.machine_token,
                "mesh",
                None, // mesh ID will be extracted from the TOML content
                content,
            )
            .await
        {
            Ok(response) => {
                tracing::info!(
                    "Mesh configuration uploaded successfully: {}, action: {}",
                    filename,
                    response.data.action
                );
                mesh_count += 1;
            }
            Err(e) => {
                tracing::error!("Failed to upload mesh configuration {}: {}", filename, e);
                // Continue with other configs rather than failing entirely
            }
        }
    }

    // Build success response
    let total_size = config_size 
        + pipeline_configs.iter().map(|(_, c)| c.len()).sum::<usize>()
        + transform_configs.iter().map(|(_, c)| c.len()).sum::<usize>()
        + mesh_configs.iter().map(|(_, c)| c.len()).sum::<usize>();
    let response = UpdateResponse {
        success: true,
        message: format!(
            "Configuration uploaded successfully (gateway: {}, pipelines: {}, transforms: {}, meshes: {})",
            gateway_id, pipeline_count, transform_count, mesh_count
        ),
        config_size: total_size,
        pipeline_count,
        transform_count,
        mesh_count,
    };

    let value = serde_json::to_value(&response).map_err(|e| {
        tracing::error!("Failed to serialize response: {}", e);
        (500, "Internal server error".to_string())
    })?;

    Ok((value, 200))
}

/// Map SDK errors to HTTP status codes and messages
fn map_sdk_error(e: runbeam_sdk::RunbeamError) -> (u16, String) {
    match e {
        runbeam_sdk::RunbeamError::Api(api_err) => match api_err {
            runbeam_sdk::ApiError::Http { status, message } => {
                (status, format!("API error: {}", message))
            }
            runbeam_sdk::ApiError::Parse(msg) => (500, format!("Parse error: {}", msg)),
            runbeam_sdk::ApiError::Request(msg) => (503, format!("Network error: {}", msg)),
            runbeam_sdk::ApiError::Network(msg) => (503, format!("Network error: {}", msg)),
        },
        runbeam_sdk::RunbeamError::Storage(msg) => (500, format!("Storage error: {}", msg)),
        _ => (500, format!("Unexpected error: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_response_serialization() {
        let response = UpdateResponse {
            success: true,
            message: "Configuration uploaded successfully".to_string(),
            config_size: 1234,
            pipeline_count: 2,
            transform_count: 3,
            mesh_count: 1,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"message\":\"Configuration uploaded successfully\""));
        assert!(json.contains("\"config_size\":1234"));
        assert!(json.contains("\"pipeline_count\":2"));
        assert!(json.contains("\"transform_count\":3"));
        assert!(json.contains("\"mesh_count\":1"));
    }

    #[test]
    fn test_update_response_deserialization() {
        let json = r#"{
            "success": true,
            "message": "Configuration uploaded successfully",
            "config_size": 5678
        }"#;

        let response: UpdateResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.success, true);
        assert_eq!(response.message, "Configuration uploaded successfully");
        assert_eq!(response.config_size, 5678);
    }
}
