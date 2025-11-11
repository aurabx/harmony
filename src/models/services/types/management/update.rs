use runbeam_sdk::{load_token, RunbeamClient};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
        .map(|config| config.proxy.id.clone())
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

    // Read TOML config file
    let toml_content = std::fs::read_to_string(&config_path).map_err(|e| {
        tracing::error!("Failed to read configuration file: {}", e);
        (500, format!("Failed to read configuration file: {}", e))
    })?;

    let config_size = toml_content.len();
    tracing::debug!("Configuration size: {} bytes", config_size);

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

    // Call store_config API to upload configuration
    // config_type = "gateway", id = gateway_id from machine token
    match client
        .store_config(
            &machine_token.machine_token,
            "gateway",
            Some(gateway_id.clone()),
            &toml_content,
        )
        .await
    {
        Ok(response) => {
            tracing::info!(
                "Configuration uploaded successfully: {} bytes, action: {}",
                config_size,
                response.data.model.action
            );

            // Build success response
            let response = UpdateResponse {
                success: true,
                message: format!(
                    "Configuration {} successfully (gateway: {})",
                    response.data.model.action, gateway_id
                ),
                config_size,
            };

            let value = serde_json::to_value(&response).map_err(|e| {
                tracing::error!("Failed to serialize response: {}", e);
                (500, "Internal server error".to_string())
            })?;

            Ok((value, 200))
        }
        Err(e) => {
            tracing::error!("Failed to upload configuration: {}", e);

            // Map SDK errors to appropriate HTTP status codes
            let (status_code, message) = match e {
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
            };

            Err((status_code, message))
        }
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
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"message\":\"Configuration uploaded successfully\""));
        assert!(json.contains("\"config_size\":1234"));
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
