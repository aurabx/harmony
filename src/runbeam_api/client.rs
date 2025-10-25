use crate::runbeam_api::types::{ApiError, AuthorizeResponse, RunbeamError};
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// HTTP client for Runbeam Cloud API
///
/// This client handles all communication with the Runbeam Cloud control plane,
/// including gateway authorization and future component loading.
#[derive(Debug, Clone)]
pub struct RunbeamClient {
    /// Base URL for the Runbeam Cloud API (from JWT iss claim)
    base_url: String,
    /// HTTP client for making requests
    client: reqwest::Client,
}

/// Request payload for gateway authorization
#[derive(Debug, Serialize)]
struct AuthorizeRequest {
    /// JWT token from the user (will be sent in body per Laravel API spec)
    token: String,
    /// Gateway code (instance ID)
    gateway_code: String,
    /// Optional machine public key for secure communication
    #[serde(skip_serializing_if = "Option::is_none")]
    machine_public_key: Option<String>,
    /// Optional metadata about the gateway
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<HashMap<String, JsonValue>>,
}

impl RunbeamClient {
    /// Create a new Runbeam Cloud API client
    ///
    /// # Arguments
    ///
    /// * `base_url` - The Runbeam Cloud API base URL (extracted from JWT iss claim)
    ///
    /// # Example
    ///
    /// ```no_run
    /// use harmony::runbeam_api::RunbeamClient;
    ///
    /// let client = RunbeamClient::new("http://runbeam.lndo.site");
    /// ```
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        tracing::debug!("Creating RunbeamClient with base URL: {}", base_url);

        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Authorize a gateway and obtain a machine-scoped token
    ///
    /// This method exchanges a user JWT token for a machine-scoped token that
    /// the gateway can use for autonomous API access. The machine token has
    /// a 30-day expiry (configured server-side).
    ///
    /// # Arguments
    ///
    /// * `user_token` - The user's JWT token from CLI authentication
    /// * `gateway_code` - The gateway instance ID
    /// * `machine_public_key` - Optional public key for secure communication
    /// * `metadata` - Optional metadata about the gateway
    ///
    /// # Returns
    ///
    /// Returns `Ok(AuthorizeResponse)` with machine token and gateway details,
    /// or `Err(RunbeamError)` if authorization fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use harmony::runbeam_api::RunbeamClient;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = RunbeamClient::new("http://runbeam.lndo.site");
    /// let response = client.authorize_gateway(
    ///     "eyJhbGci...",
    ///     "gateway-123",
    ///     None,
    ///     None
    /// ).await?;
    ///
    /// println!("Machine token: {}", response.machine_token);
    /// println!("Expires at: {}", response.expires_at);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn authorize_gateway(
        &self,
        user_token: impl Into<String>,
        gateway_code: impl Into<String>,
        machine_public_key: Option<String>,
        metadata: Option<HashMap<String, JsonValue>>,
    ) -> Result<AuthorizeResponse, RunbeamError> {
        let user_token = user_token.into();
        let gateway_code = gateway_code.into();

        tracing::info!(
            "Authorizing gateway with Runbeam Cloud: gateway_code={}",
            gateway_code
        );

        // Construct the authorization endpoint URL
        let url = format!("{}/api/harmony/authorize", self.base_url);

        // Build request payload
        let payload = AuthorizeRequest {
            token: user_token.clone(),
            gateway_code: gateway_code.clone(),
            machine_public_key,
            metadata,
        };

        tracing::debug!("Sending authorization request to: {}", url);

        // Make the request
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", user_token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("Failed to send authorization request: {}", e);
                ApiError::from(e)
            })?;

        let status = response.status();
        tracing::debug!("Received response with status: {}", status);

        // Handle error responses
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            tracing::error!(
                "Authorization failed: HTTP {} - {}",
                status.as_u16(),
                error_body
            );

            return Err(RunbeamError::Api(ApiError::Http {
                status: status.as_u16(),
                message: error_body,
            }));
        }

        // Parse successful response
        let auth_response: AuthorizeResponse = response.json().await.map_err(|e| {
            tracing::error!("Failed to parse authorization response: {}", e);
            ApiError::Parse(format!("Failed to parse response JSON: {}", e))
        })?;

        tracing::info!(
            "Gateway authorized successfully: gateway_id={}, expires_at={}",
            auth_response.gateway.id,
            auth_response.expires_at
        );

        tracing::debug!("Machine token length: {}", auth_response.machine_token.len());
        tracing::debug!("Gateway abilities: {:?}", auth_response.abilities);

        Ok(auth_response)
    }

    /// Get the base URL for this client
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = RunbeamClient::new("http://example.com");
        assert_eq!(client.base_url(), "http://example.com");
    }

    #[test]
    fn test_client_creation_with_string() {
        let base_url = String::from("http://example.com");
        let client = RunbeamClient::new(base_url);
        assert_eq!(client.base_url(), "http://example.com");
    }

    #[test]
    fn test_authorize_request_serialization() {
        let request = AuthorizeRequest {
            token: "test_token".to_string(),
            gateway_code: "gw123".to_string(),
            machine_public_key: Some("pubkey123".to_string()),
            metadata: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"token\":\"test_token\""));
        assert!(json.contains("\"gateway_code\":\"gw123\""));
        assert!(json.contains("\"machine_public_key\":\"pubkey123\""));
    }

    #[test]
    fn test_authorize_request_serialization_without_optional_fields() {
        let request = AuthorizeRequest {
            token: "test_token".to_string(),
            gateway_code: "gw123".to_string(),
            machine_public_key: None,
            metadata: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"token\":\"test_token\""));
        assert!(json.contains("\"gateway_code\":\"gw123\""));
        // Should not contain the optional fields
        assert!(!json.contains("machine_public_key"));
        assert!(!json.contains("metadata"));
    }
}
