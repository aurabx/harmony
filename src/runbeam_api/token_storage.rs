use crate::storage::{StorageBackend, StorageError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Path where machine tokens are stored relative to storage root
const TOKEN_STORAGE_PATH: &str = "runbeam/auth.json";

/// Machine-scoped token for Runbeam Cloud API authentication
///
/// This token is issued by Runbeam Cloud and allows the gateway to make
/// autonomous API calls without user intervention. It has a 30-day expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineToken {
    /// The machine token string
    pub machine_token: String,
    /// When the token expires (ISO 8601 format)
    pub expires_at: String,
    /// Gateway ID
    pub gateway_id: String,
    /// Gateway code (instance ID)
    pub gateway_code: String,
    /// Token abilities/permissions
    #[serde(default)]
    pub abilities: Vec<String>,
    /// When this token was issued/stored (ISO 8601 format)
    pub issued_at: String,
}

impl MachineToken {
    /// Create a new machine token
    pub fn new(
        machine_token: String,
        expires_at: String,
        gateway_id: String,
        gateway_code: String,
        abilities: Vec<String>,
    ) -> Self {
        let issued_at = Utc::now().to_rfc3339();

        Self {
            machine_token,
            expires_at,
            gateway_id,
            gateway_code,
            abilities,
            issued_at,
        }
    }

    /// Check if the token has expired
    pub fn is_expired(&self) -> bool {
        // Parse the expiry timestamp
        match DateTime::parse_from_rfc3339(&self.expires_at) {
            Ok(expiry) => {
                let now = Utc::now();
                expiry.with_timezone(&Utc) < now
            }
            Err(e) => {
                tracing::warn!("Failed to parse token expiry date: {}", e);
                // If we can't parse the date, consider it expired for safety
                true
            }
        }
    }

    /// Check if the token is still valid (not expired)
    pub fn is_valid(&self) -> bool {
        !self.is_expired()
    }
}

/// Save a machine token to storage
///
/// Stores the token at `runbeam/auth.json` relative to the storage backend root.
///
/// # Arguments
///
/// * `storage` - The storage backend to use
/// * `token` - The machine token to save
///
/// # Returns
///
/// Returns `Ok(())` if the token was saved successfully, or `Err(StorageError)`
/// if the operation failed.
pub async fn save_token(
    storage: &dyn StorageBackend,
    token: &MachineToken,
) -> Result<(), StorageError> {
    tracing::debug!(
        "Saving machine token for gateway: {}",
        token.gateway_code
    );

    // Serialize token to JSON
    let json = serde_json::to_vec_pretty(&token).map_err(|e| {
        tracing::error!("Failed to serialize machine token: {}", e);
        StorageError::Config(format!("JSON serialization failed: {}", e))
    })?;

    // Write to storage
    storage.write_file_str(TOKEN_STORAGE_PATH, &json).await?;

    tracing::info!(
        "Machine token saved successfully: gateway_id={}, expires_at={}",
        token.gateway_id,
        token.expires_at
    );

    Ok(())
}

/// Load a machine token from storage
///
/// Attempts to load the token from `runbeam/auth.json`. Returns `None` if the
/// file doesn't exist.
///
/// # Arguments
///
/// * `storage` - The storage backend to use
///
/// # Returns
///
/// Returns `Ok(Some(token))` if a token was found and loaded successfully,
/// `Ok(None)` if no token file exists, or `Err(StorageError)` if loading failed.
pub async fn load_token(
    storage: &dyn StorageBackend,
) -> Result<Option<MachineToken>, StorageError> {
    tracing::debug!("Loading machine token from storage");

    // Check if token file exists
    if !storage.exists_str(TOKEN_STORAGE_PATH) {
        tracing::debug!("No machine token file found");
        return Ok(None);
    }

    // Read token file
    let json = storage.read_file_str(TOKEN_STORAGE_PATH).await?;

    // Deserialize token
    let token: MachineToken = serde_json::from_slice(&json).map_err(|e| {
        tracing::error!("Failed to deserialize machine token: {}", e);
        StorageError::Config(format!("JSON deserialization failed: {}", e))
    })?;

    tracing::debug!(
        "Machine token loaded: gateway_id={}, expires_at={}, valid={}",
        token.gateway_id,
        token.expires_at,
        token.is_valid()
    );

    Ok(Some(token))
}

/// Clear the machine token from storage
///
/// Removes the token file at `runbeam/auth.json` if it exists.
///
/// # Arguments
///
/// * `storage` - The storage backend to use
///
/// # Returns
///
/// Returns `Ok(())` if the token was cleared successfully or didn't exist,
/// or `Err(StorageError)` if the operation failed.
pub async fn clear_token(storage: &dyn StorageBackend) -> Result<(), StorageError> {
    tracing::debug!("Clearing machine token from storage");

    // Check if token file exists
    if !storage.exists_str(TOKEN_STORAGE_PATH) {
        tracing::debug!("No machine token file to clear");
        return Ok(());
    }

    // Remove the token file
    storage.remove_str(TOKEN_STORAGE_PATH).await?;

    tracing::info!("Machine token cleared successfully");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FilesystemStorage;
    use tempfile::TempDir;

    #[test]
    fn test_machine_token_creation() {
        let token = MachineToken::new(
            "test_token".to_string(),
            "2025-12-31T23:59:59Z".to_string(),
            "gw123".to_string(),
            "gateway-code-123".to_string(),
            vec!["harmony:send".to_string(), "harmony:receive".to_string()],
        );

        assert_eq!(token.machine_token, "test_token");
        assert_eq!(token.gateway_id, "gw123");
        assert_eq!(token.gateway_code, "gateway-code-123");
        assert_eq!(token.abilities.len(), 2);
        assert!(!token.issued_at.is_empty());
    }

    #[test]
    fn test_machine_token_is_expired() {
        // Expired token (year 2020)
        let expired_token = MachineToken::new(
            "test_token".to_string(),
            "2020-01-01T00:00:00Z".to_string(),
            "gw123".to_string(),
            "gateway-code-123".to_string(),
            vec![],
        );
        assert!(expired_token.is_expired());
        assert!(!expired_token.is_valid());

        // Valid token (far future)
        let valid_token = MachineToken::new(
            "test_token".to_string(),
            "2099-12-31T23:59:59Z".to_string(),
            "gw123".to_string(),
            "gateway-code-123".to_string(),
            vec![],
        );
        assert!(!valid_token.is_expired());
        assert!(valid_token.is_valid());
    }

    #[test]
    fn test_machine_token_serialization() {
        let token = MachineToken::new(
            "test_token".to_string(),
            "2025-12-31T23:59:59Z".to_string(),
            "gw123".to_string(),
            "gateway-code-123".to_string(),
            vec!["harmony:send".to_string()],
        );

        let json = serde_json::to_string(&token).unwrap();
        assert!(json.contains("\"machine_token\":\"test_token\""));
        assert!(json.contains("\"gateway_id\":\"gw123\""));
        assert!(json.contains("\"gateway_code\":\"gateway-code-123\""));

        // Deserialize and verify
        let deserialized: MachineToken = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.machine_token, token.machine_token);
        assert_eq!(deserialized.gateway_id, token.gateway_id);
    }

    #[tokio::test]
    async fn test_save_and_load_token() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path()).unwrap();

        let token = MachineToken::new(
            "test_token".to_string(),
            "2025-12-31T23:59:59Z".to_string(),
            "gw123".to_string(),
            "gateway-code-123".to_string(),
            vec!["harmony:send".to_string()],
        );

        // Save token
        save_token(&storage, &token).await.unwrap();

        // Load token
        let loaded = load_token(&storage).await.unwrap();
        assert!(loaded.is_some());

        let loaded_token = loaded.unwrap();
        assert_eq!(loaded_token.machine_token, token.machine_token);
        assert_eq!(loaded_token.gateway_id, token.gateway_id);
        assert_eq!(loaded_token.gateway_code, token.gateway_code);
    }

    #[tokio::test]
    async fn test_load_nonexistent_token() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path()).unwrap();

        // Load from empty storage
        let result = load_token(&storage).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_clear_token() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path()).unwrap();

        let token = MachineToken::new(
            "test_token".to_string(),
            "2025-12-31T23:59:59Z".to_string(),
            "gw123".to_string(),
            "gateway-code-123".to_string(),
            vec![],
        );

        // Save token
        save_token(&storage, &token).await.unwrap();

        // Verify it exists
        assert!(load_token(&storage).await.unwrap().is_some());

        // Clear token
        clear_token(&storage).await.unwrap();

        // Verify it's gone
        assert!(load_token(&storage).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_clear_nonexistent_token() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path()).unwrap();

        // Clear non-existent token should not error
        clear_token(&storage).await.unwrap();
    }
}
