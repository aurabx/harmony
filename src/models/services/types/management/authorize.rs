use crate::adapters::registry::AdapterRegistry;
use runbeam_sdk::{
    extract_bearer_token, save_token, save_token_with_key, validate_jwt_token, ApiError,
    MachineToken, RunbeamClient, RunbeamError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Request body for gateway authorization
#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    /// Gateway code (instance ID)
    pub gateway_code: String,
    /// Optional machine public key for secure communication
    #[serde(default)]
    pub machine_public_key: Option<String>,
    /// Optional metadata about the gateway
    #[serde(default)]
    pub metadata: Option<HashMap<String, JsonValue>>,
    /// Optional encryption key for token storage (base64-encoded age X25519 key)
    /// If provided, this key will be used to encrypt the machine token.
    /// If not provided, Harmony will use RUNBEAM_ENCRYPTION_KEY env var or auto-generate a key.
    #[serde(default)]
    pub encryption_key: Option<String>,
}

/// Response for successful authorization
#[derive(Debug, Serialize)]
pub struct AuthorizeResponse {
    /// Success status
    pub success: bool,
    /// Message describing the result
    pub message: String,
    /// Gateway details
    pub gateway: GatewayDetails,
    /// When the machine token expires
    pub expires_at: String,
    /// Seconds until expiry
    pub expires_in: i64,
}

#[derive(Debug, Serialize)]
pub struct GatewayDetails {
    pub id: String,
    pub code: String,
    pub name: String,
}

/// Error response for authorization failures
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

/// Handle gateway authorization request
///
/// This endpoint:
/// 1. Extracts and validates the JWT token from Authorization header (RS256)
/// 2. Calls Runbeam Cloud API to exchange user token for machine token
/// 3. Stores the machine token locally
/// 4. Starts cloud polling immediately
/// 5. Returns success response with gateway details
pub async fn handle_authorize(
    auth_header: Option<&str>,
    body: &[u8],
    jwks_cache_duration_hours: u64,
    registry: Arc<AdapterRegistry>,
    poll_interval: Duration,
    api_base_url_override: String,
) -> Result<serde_json::Value, (u16, String)> {
    tracing::info!("Processing gateway authorization request");

    // Extract JWT token from Authorization header
    let auth_header = auth_header.ok_or_else(|| {
        tracing::warn!("Missing Authorization header");
        (401, "Missing Authorization header".to_string())
    })?;

    let user_token = extract_bearer_token(auth_header).map_err(|e| {
        tracing::error!("Failed to extract Bearer token: {}", e);
        (401, format!("Invalid Authorization header: {}", e))
    })?;

    tracing::debug!("Extracted JWT token from Authorization header");

    // Parse request body
    let request: AuthorizeRequest = serde_json::from_slice(body).map_err(|e| {
        tracing::error!("Failed to parse request body: {}", e);
        (400, format!("Invalid request body: {}", e))
    })?;

    tracing::info!("Authorizing gateway: {}", request.gateway_code);

    // Validate JWT using RS256 with JWKS
    let _claims = validate_jwt_token(user_token, jwks_cache_duration_hours)
        .await
        .map_err(|e| {
            tracing::error!("JWT validation failed: {}", e);
            (401, format!("Invalid or expired token: {}", e))
        })?;

    // Use the override API base URL provided by the caller (from config)
    // This is the base URL that will be used for both the authorization call and cloud polling
    let api_base_url = api_base_url_override;
    tracing::debug!("Runbeam API base URL: {}", api_base_url);

    // Create Runbeam Cloud API client
    let client = RunbeamClient::new(api_base_url.clone());

    // Call Runbeam Cloud API to authorize gateway
    let auth_response = client
        .authorize_gateway(
            user_token,
            &request.gateway_code,
            request.machine_public_key.clone(),
            request
                .metadata
                .as_ref()
                .map(|m| m.keys().cloned().collect()),
        )
        .await
        .map_err(|e| {
            tracing::error!("Runbeam Cloud authorization failed: {}", e);

            // Map error to appropriate HTTP status code
            let status_code = match &e {
                RunbeamError::JwtValidation(_) => 401,
                RunbeamError::Api(api_err) => match api_err {
                    ApiError::Http { status, .. } => *status,
                    _ => 500,
                },
                _ => 500,
            };

            (status_code, format!("Authorization failed: {}", e))
        })?;

    tracing::info!(
        "Successfully authorized with Runbeam Cloud: gateway_id={}",
        auth_response.gateway.id
    );

    // Create machine token for storage
    let machine_token = MachineToken::new(
        auth_response.machine_token.clone(),
        auth_response.expires_at.clone(),
        auth_response.gateway.id.clone(),
        auth_response.gateway.code.clone(),
        auth_response.abilities.clone(),
    );

    // Get proxy ID for instance isolation
    let proxy_id = crate::globals::get_config()
        .map(|config| config.proxy.id.clone())
        .unwrap_or_else(|| "harmony".to_string());

    // Save machine token using appropriate method based on whether encryption key was provided
    if let Some(ref encryption_key) = request.encryption_key {
        tracing::debug!("Using encryption key provided by CLI");
        save_token_with_key(&proxy_id, &machine_token, encryption_key)
            .await
            .map_err(|e| {
                tracing::error!("Failed to save machine token with provided key: {}", e);
                (500, format!("Failed to save token: {}", e))
            })?;
    } else {
        tracing::debug!("No encryption key provided, using default key management");
        save_token(&proxy_id, "auth", &machine_token)
            .await
            .map_err(|e| {
                tracing::error!("Failed to save machine token: {}", e);
                (500, format!("Failed to save token: {}", e))
            })?;
    }

    tracing::info!("Machine token saved successfully");

    // Start cloud polling with the new machine token
    start_cloud_polling_task(
        api_base_url,
        machine_token.machine_token.clone(),
        poll_interval,
        registry,
    );

    // Build success response
    let response = AuthorizeResponse {
        success: true,
        message: "Gateway authorized successfully".to_string(),
        gateway: GatewayDetails {
            id: auth_response.gateway.id,
            code: auth_response.gateway.code,
            name: auth_response.gateway.name,
        },
        expires_at: auth_response.expires_at,
        expires_in: auth_response.expires_in as i64,
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
    fn test_authorize_request_deserialization() {
        let json = r#"{
            "gateway_code": "test-gateway-123"
        }"#;

        let request: AuthorizeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.gateway_code, "test-gateway-123");
        assert!(request.machine_public_key.is_none());
        assert!(request.metadata.is_none());
        assert!(request.encryption_key.is_none());
    }

    #[test]
    fn test_authorize_request_with_optional_fields() {
        let json = r#"{
            "gateway_code": "test-gateway-123",
            "machine_public_key": "pubkey123",
            "metadata": {
                "version": "0.4.0",
                "os": "macos"
            },
            "encryption_key": "QUdFLVNFQ1JFVC1LRVktMTIzNDU2Nzg5MA=="
        }"#;

        let request: AuthorizeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.gateway_code, "test-gateway-123");
        assert_eq!(request.machine_public_key.as_deref(), Some("pubkey123"));
        assert!(request.metadata.is_some());
        assert_eq!(
            request.encryption_key.as_deref(),
            Some("QUdFLVNFQ1JFVC1LRVktMTIzNDU2Nzg5MA==")
        );
    }

    #[test]
    fn test_authorize_response_serialization() {
        let response = AuthorizeResponse {
            success: true,
            message: "Gateway authorized successfully".to_string(),
            gateway: GatewayDetails {
                id: "gw123".to_string(),
                code: "test-gateway".to_string(),
                name: "Test Gateway".to_string(),
            },
            expires_at: "2025-12-31T23:59:59Z".to_string(),
            expires_in: 2592000,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"code\":\"test-gateway\""));
    }

    #[test]
    fn test_error_response_serialization() {
        let error = ErrorResponse {
            error: "Unauthorized".to_string(),
            message: "Invalid or expired token".to_string(),
        };

        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"error\":\"Unauthorized\""));
        assert!(json.contains("\"message\":\"Invalid or expired token\""));
    }
}

/// Start cloud polling task in the background
///
/// This function spawns a tokio task that continuously polls Runbeam Cloud
/// for pending config changes.
fn start_cloud_polling_task(
    api_base_url: String,
    machine_token: String,
    poll_interval: Duration,
    registry: Arc<AdapterRegistry>,
) {
    tracing::info!(
        "🌥️  Starting cloud config polling after successful authorization (interval: {:?})",
        poll_interval
    );

    // Create a new cancellation token for this polling session
    let shutdown = tokio_util::sync::CancellationToken::new();

    // Store the cancellation token globally so it can be cancelled later
    crate::globals::set_cloud_polling_token(shutdown.clone());

    // Create Runbeam API client
    let client = RunbeamClient::new(api_base_url);

    // Spawn the cloud polling task
    tokio::spawn(async move {
        super::cloud_poller::start_cloud_polling(
            client,
            machine_token,
            poll_interval,
            registry,
            shutdown,
        )
        .await;
    });
}
