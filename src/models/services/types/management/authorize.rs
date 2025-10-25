use crate::runbeam_api::{jwt, token_storage, RunbeamClient};
use crate::runbeam_api::token_storage::MachineToken;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

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
/// 1. Extracts and validates the JWT token from Authorization header
/// 2. Calls Runbeam Cloud API to exchange user token for machine token
/// 3. Stores the machine token locally
/// 4. Returns success response with gateway details
pub async fn handle_authorize(
    auth_header: Option<&str>,
    body: &[u8],
) -> Result<serde_json::Value, (u16, String)> {
    tracing::info!("Processing gateway authorization request");

    // Extract JWT token from Authorization header
    let auth_header = auth_header.ok_or_else(|| {
        tracing::warn!("Missing Authorization header");
        (401, "Missing Authorization header".to_string())
    })?;

    let user_token = jwt::extract_bearer_token(auth_header).map_err(|e| {
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

    // Validate JWT and extract claims
    // TODO: Get JWT secret from configuration (currently using placeholder)
    let jwt_secret = get_jwt_secret();
    let claims = jwt::validate_jwt_token(user_token, jwt_secret.as_bytes()).map_err(|e| {
        tracing::error!("JWT validation failed: {}", e);
        (401, format!("Invalid or expired token: {}", e))
    })?;

    // Extract Runbeam API base URL from JWT issuer claim
    let api_base_url = claims.api_base_url();
    tracing::debug!("Runbeam API base URL: {}", api_base_url);

    // Create Runbeam Cloud API client
    let client = RunbeamClient::new(api_base_url);

    // Call Runbeam Cloud API to authorize gateway
    let auth_response = client
        .authorize_gateway(
            user_token,
            &request.gateway_code,
            request.machine_public_key.clone(),
            request.metadata.clone(),
        )
        .await
        .map_err(|e| {
            tracing::error!("Runbeam Cloud authorization failed: {}", e);
            
            // Map error to appropriate HTTP status code
            let status_code = match &e {
                crate::runbeam_api::types::RunbeamError::JwtValidation(_) => 401,
                crate::runbeam_api::types::RunbeamError::Api(api_err) => {
                    match api_err {
                        crate::runbeam_api::types::ApiError::Http { status, .. } => *status,
                        _ => 500,
                    }
                }
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

    // Save machine token to storage
    let storage = crate::globals::get_storage().ok_or_else(|| {
        tracing::error!("Storage backend not initialized");
        (500, "Internal server error: storage not available".to_string())
    })?;

    token_storage::save_token(storage.as_ref(), &machine_token)
        .await
        .map_err(|e| {
            tracing::error!("Failed to save machine token: {}", e);
            (500, format!("Failed to save token: {}", e))
        })?;

    tracing::info!("Machine token saved successfully");

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

/// Get the JWT secret for validation
///
/// TODO: This should come from configuration. For now, it's hardcoded to match
/// the default JWT_SECRET used by Runbeam Cloud in development.
///
/// In production, this should be:
/// 1. Read from config file ([runbeam] section)
/// 2. Or from environment variable
/// 3. Must match the secret used by Runbeam Cloud to sign JWTs
fn get_jwt_secret() -> String {
    // Check environment variable first
    if let Ok(secret) = std::env::var("RUNBEAM_JWT_SECRET") {
        tracing::debug!("Using JWT secret from RUNBEAM_JWT_SECRET environment variable");
        return secret;
    }

    // Fallback to development default
    // This matches the default JWT_SECRET in Runbeam Cloud's .env.example
    tracing::warn!("Using default JWT secret for development. Set RUNBEAM_JWT_SECRET environment variable for production.");
    "base64:your-secret-key-goes-here".to_string()
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
    }

    #[test]
    fn test_authorize_request_with_optional_fields() {
        let json = r#"{
            "gateway_code": "test-gateway-123",
            "machine_public_key": "pubkey123",
            "metadata": {
                "version": "0.4.0",
                "os": "macos"
            }
        }"#;

        let request: AuthorizeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.gateway_code, "test-gateway-123");
        assert_eq!(request.machine_public_key.as_deref(), Some("pubkey123"));
        assert!(request.metadata.is_some());
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
        assert!(json.contains("\"gateway_code\":\"test-gateway\""));
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
