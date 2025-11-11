use runbeam_sdk::{save_token, MachineToken};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Request body for setting machine token
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    /// Machine token from Runbeam Cloud API
    pub machine_token: String,
    /// When the token expires (ISO 8601 format)
    pub expires_at: String,
    /// Gateway ID
    pub gateway_id: String,
    /// Gateway code
    pub gateway_code: String,
    /// Token abilities/scopes
    #[serde(default)]
    pub abilities: Vec<String>,
    /// Optional encryption key for token storage (base64-encoded)
    #[serde(default)]
    pub encryption_key: Option<String>,
}

/// Response for successful token save
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    /// Success status
    pub success: bool,
    /// Message describing the result
    pub message: String,
}

/// Handle POST request to save machine token
///
/// This endpoint allows the runbeam CLI to send a machine token
/// directly to harmony-proxy after successful API authorization.
///
/// Flow:
/// 1. CLI calls Runbeam Cloud API to authorize gateway
/// 2. API returns machine token
/// 3. CLI POSTs machine token to this endpoint
/// 4. Harmony-proxy saves token to local storage
pub async fn handle_token_post(body: &[u8]) -> Result<JsonValue, (u16, String)> {
    tracing::info!("Processing token save request from CLI");

    // Parse request body
    let request: TokenRequest = serde_json::from_slice(body).map_err(|e| {
        tracing::error!("Failed to parse token request body: {}", e);
        (400, format!("Invalid request body: {}", e))
    })?;

    // Validate token is not empty
    if request.machine_token.is_empty() {
        tracing::warn!("Received empty machine token");
        return Err((400, "Machine token cannot be empty".to_string()));
    }

    if request.gateway_id.is_empty() {
        tracing::warn!("Received empty gateway_id");
        return Err((400, "Gateway ID cannot be empty".to_string()));
    }

    tracing::info!(
        "Saving machine token for gateway: {} ({})",
        request.gateway_code,
        request.gateway_id
    );

    // If encryption key is provided, set it as environment variable
    // The SDK will use this key for encrypting the token
    if let Some(key) = request.encryption_key {
        tracing::info!("Using provided encryption key for token storage");
        std::env::set_var("RUNBEAM_ENCRYPTION_KEY", &key);
    }

    // Create machine token struct for storage
    let machine_token = MachineToken::new(
        request.machine_token,
        request.expires_at,
        request.gateway_id,
        request.gateway_code,
        request.abilities,
    );

    // Get proxy ID for instance isolation
    let proxy_id = crate::globals::get_config()
        .map(|config| config.proxy.id.clone())
        .unwrap_or_else(|| "harmony".to_string());

    // Save token to secure storage (SDK manages keyring/encrypted filesystem automatically)
    save_token(&proxy_id, "auth", &machine_token)
        .await
        .map_err(|e| {
            tracing::error!("Failed to save machine token: {}", e);
            (500, format!("Failed to save token: {}", e))
        })?;

    tracing::info!("Machine token saved successfully");

    // Build success response
    let response = TokenResponse {
        success: true,
        message: "Machine token saved successfully".to_string(),
    };

    serde_json::to_value(&response).map_err(|e| {
        tracing::error!("Failed to serialize response: {}", e);
        (500, "Internal server error".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_request_deserialization() {
        let json = r#"{
            "machine_token": "mt_abc123",
            "expires_at": "2025-11-30T00:00:00Z",
            "gateway_id": "gw_123",
            "gateway_code": "test-gateway"
        }"#;

        let request: TokenRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.machine_token, "mt_abc123");
        assert_eq!(request.expires_at, "2025-11-30T00:00:00Z");
        assert_eq!(request.gateway_id, "gw_123");
        assert_eq!(request.gateway_code, "test-gateway");
        assert!(request.abilities.is_empty());
    }

    #[test]
    fn test_token_request_with_abilities() {
        let json = r#"{
            "machine_token": "mt_abc123",
            "expires_at": "2025-11-30T00:00:00Z",
            "gateway_id": "gw_123",
            "gateway_code": "test-gateway",
            "abilities": ["gateway:read", "gateway:write"]
        }"#;

        let request: TokenRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.abilities.len(), 2);
        assert_eq!(request.abilities[0], "gateway:read");
    }

    #[test]
    fn test_token_response_serialization() {
        let response = TokenResponse {
            success: true,
            message: "Machine token saved successfully".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"message\":\"Machine token saved successfully\""));
    }
}
