use crate::error::{JmixError, JmixResult};
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::path::Path;

/// JWS Header for Ed25519 signatures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwsHeader {
    /// Algorithm - always "EdDSA" for Ed25519
    pub alg: String,
    /// Type - always "JWS" 
    pub typ: String,
    /// Curve - "Ed25519" for our implementation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crv: Option<String>,
}

impl Default for JwsHeader {
    fn default() -> Self {
        Self {
            alg: "EdDSA".to_string(),
            typ: "JWS".to_string(),
            crv: Some("Ed25519".to_string()),
        }
    }
}

/// JWS (JSON Web Signature) implementation for JMIX envelopes
pub struct JwsManager {
    signing_key: Option<SigningKey>,
}

impl JwsManager {
    /// Create a new JWS manager without a signing key
    pub fn new() -> Self {
        Self {
            signing_key: None,
        }
    }

    /// Create a new JWS manager with a generated signing key
    pub fn with_generated_key() -> JmixResult<Self> {
        let mut secret_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        Ok(Self {
            signing_key: Some(signing_key),
        })
    }

    /// Create a JWS manager with a signing key loaded from file
    pub fn with_key_file<P: AsRef<Path>>(key_path: P) -> JmixResult<Self> {
        let key_bytes = fs::read(key_path.as_ref())
            .map_err(|e| JmixError::Io(e))?;
        
        if key_bytes.len() != 32 {
            return Err(JmixError::Cryptography(
                crate::error::CryptographyError::InvalidKey(
                    format!("Ed25519 private key must be 32 bytes, got {}", key_bytes.len())
                )
            ));
        }

        let key_array: [u8; 32] = key_bytes.try_into()
            .map_err(|_| JmixError::Cryptography(
                crate::error::CryptographyError::InvalidKey(
                    "Failed to convert key bytes to array".to_string()
                )
            ))?;

        let signing_key = SigningKey::from_bytes(&key_array);
        Ok(Self {
            signing_key: Some(signing_key),
        })
    }

    /// Create a JWS manager with a signing key from raw bytes
    pub fn with_key_bytes(key_bytes: &[u8; 32]) -> JmixResult<Self> {
        let signing_key = SigningKey::from_bytes(key_bytes);
        Ok(Self {
            signing_key: Some(signing_key),
        })
    }

    /// Generate a new Ed25519 key pair and save to files
    pub fn generate_keypair<P: AsRef<Path>>(
        private_key_path: P,
        public_key_path: P,
    ) -> JmixResult<Self> {
        let mut secret_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();

        // Save private key (32 bytes)
        fs::write(&private_key_path, signing_key.to_bytes())
            .map_err(|e| JmixError::Io(e))?;

        // Save public key (32 bytes) 
        fs::write(&public_key_path, verifying_key.to_bytes())
            .map_err(|e| JmixError::Io(e))?;

        println!("Generated Ed25519 keypair:");
        println!("  Private key: {}", private_key_path.as_ref().display());
        println!("  Public key: {}", public_key_path.as_ref().display());

        Ok(Self {
            signing_key: Some(signing_key),
        })
    }

    /// Get the verifying key (public key) if a signing key is loaded
    pub fn verifying_key(&self) -> Option<VerifyingKey> {
        self.signing_key.as_ref().map(|sk| sk.verifying_key())
    }

    /// Get the public key bytes if a signing key is loaded
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.signing_key
            .as_ref()
            .map(|sk| sk.verifying_key().to_bytes().to_vec())
            .unwrap_or_default()
    }

    /// Sign raw data and return the signature bytes
    pub fn sign_data(&self, data: &[u8]) -> JmixResult<Vec<u8>> {
        let signing_key = self.signing_key.as_ref()
            .ok_or_else(|| JmixError::Cryptography(
                crate::error::CryptographyError::JwsCreation(
                    "No signing key loaded".to_string()
                )
            ))?;

        let signature = signing_key.sign(data);
        Ok(signature.to_bytes().to_vec())
    }

    /// Sign a JSON payload and return JWS compact serialization
    pub fn sign_json(&self, payload: &str) -> JmixResult<String> {
        let signing_key = self.signing_key.as_ref()
            .ok_or_else(|| JmixError::Cryptography(
                crate::error::CryptographyError::JwsCreation(
                    "No signing key loaded".to_string()
                )
            ))?;

        let header = JwsHeader::default();
        let header_json = serde_json::to_string(&header)
            .map_err(|e| JmixError::Json(e))?;

        // Create JWS signing input: base64url(header) + "." + base64url(payload)
        let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(header_json.as_bytes());
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(payload.as_bytes());
        
        let signing_input = format!("{}.{}", header_b64, payload_b64);

        // Sign the input
        let signature = signing_key.sign(signing_input.as_bytes());
        let signature_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(signature.to_bytes());

        // Return JWS compact serialization
        Ok(format!("{}.{}", signing_input, signature_b64))
    }

    /// Verify a JWS compact serialization against a public key
    pub fn verify_jws(jws: &str, verifying_key: &VerifyingKey) -> JmixResult<String> {
        let parts: Vec<&str> = jws.split('.').collect();
        if parts.len() != 3 {
            return Err(JmixError::Cryptography(
                crate::error::CryptographyError::JwsVerification(
                    "Invalid JWS format - expected 3 parts".to_string()
                )
            ));
        }

        let header_b64 = parts[0];
        let payload_b64 = parts[1];
        let signature_b64 = parts[2];

        // Reconstruct signing input
        let signing_input = format!("{}.{}", header_b64, payload_b64);

        // Decode and verify signature
        let signature_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature_b64)
            .map_err(|e| JmixError::Cryptography(
                crate::error::CryptographyError::JwsVerification(
                    format!("Invalid base64 signature: {}", e)
                )
            ))?;

        if signature_bytes.len() != 64 {
            return Err(JmixError::Cryptography(
                crate::error::CryptographyError::JwsVerification(
                    format!("Invalid signature length: expected 64 bytes, got {}", signature_bytes.len())
                )
            ));
        }

        let signature = Signature::from_bytes(&signature_bytes.try_into().unwrap());

        // Verify signature
        verifying_key
            .verify(signing_input.as_bytes(), &signature)
            .map_err(|e| JmixError::Cryptography(
                crate::error::CryptographyError::JwsVerification(
                    format!("Signature verification failed: {}", e)
                )
            ))?;

        // Decode and return payload
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64)
            .map_err(|e| JmixError::Cryptography(
                crate::error::CryptographyError::JwsVerification(
                    format!("Invalid base64 payload: {}", e)
                )
            ))?;

        String::from_utf8(payload_bytes)
            .map_err(|e| JmixError::Cryptography(
                crate::error::CryptographyError::JwsVerification(
                    format!("Invalid UTF-8 in payload: {}", e)
                )
            ))
    }

    /// Load a public key from file for verification
    pub fn load_public_key<P: AsRef<Path>>(public_key_path: P) -> JmixResult<VerifyingKey> {
        let key_bytes = fs::read(public_key_path.as_ref())
            .map_err(|e| JmixError::Io(e))?;
        
        if key_bytes.len() != 32 {
            return Err(JmixError::Cryptography(
                crate::error::CryptographyError::InvalidKey(
                    format!("Ed25519 public key must be 32 bytes, got {}", key_bytes.len())
                )
            ));
        }

        let key_array: [u8; 32] = key_bytes.try_into()
            .map_err(|_| JmixError::Cryptography(
                crate::error::CryptographyError::InvalidKey(
                    "Failed to convert public key bytes to array".to_string()
                )
            ))?;

        Ok(VerifyingKey::from_bytes(&key_array)
            .map_err(|e| JmixError::Cryptography(
                crate::error::CryptographyError::InvalidKey(
                    format!("Invalid Ed25519 public key: {}", e)
                )
            ))?)
    }
}

impl Default for JwsManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_jws_manager_creation() {
        let manager = JwsManager::new();
        assert!(manager.signing_key.is_none());
    }

    #[test]
    fn test_key_generation() {
        let manager = JwsManager::with_generated_key().unwrap();
        assert!(manager.signing_key.is_some());
        assert!(manager.verifying_key().is_some());
    }

    #[test]
    fn test_sign_and_verify() {
        let manager = JwsManager::with_generated_key().unwrap();
        let verifying_key = manager.verifying_key().unwrap();
        
        let payload = r#"{"test": "data", "number": 42}"#;
        let jws = manager.sign_json(payload).unwrap();
        
        // JWS should have 3 parts separated by dots
        assert_eq!(jws.split('.').count(), 3);
        
        // Verify the signature
        let verified_payload = JwsManager::verify_jws(&jws, &verifying_key).unwrap();
        assert_eq!(verified_payload, payload);
    }

    #[test]
    fn test_keypair_file_generation() {
        let temp_dir = TempDir::new().unwrap();
        let private_key_path = temp_dir.path().join("private.key");
        let public_key_path = temp_dir.path().join("public.key");

        // Generate keypair files
        let _manager = JwsManager::generate_keypair(&private_key_path, &public_key_path).unwrap();

        // Files should exist after generation
        assert!(private_key_path.exists());
        assert!(public_key_path.exists());
        
        // Keys should be 32 bytes each
        assert_eq!(fs::read(&private_key_path).unwrap().len(), 32);
        assert_eq!(fs::read(&public_key_path).unwrap().len(), 32);
        
        // Should be able to load the key back
        let loaded_manager = JwsManager::with_key_file(&private_key_path).unwrap();
        let loaded_public_key = JwsManager::load_public_key(&public_key_path).unwrap();
        
        // Should be able to sign and verify with loaded keys
        let payload = r#"{"loaded": "test"}"#;
        let jws = loaded_manager.sign_json(payload).unwrap();
        let verified = JwsManager::verify_jws(&jws, &loaded_public_key).unwrap();
        assert_eq!(verified, payload);
    }

    #[test]
    fn test_invalid_jws_format() {
        let manager = JwsManager::with_generated_key().unwrap();
        let verifying_key = manager.verifying_key().unwrap();
        
        // Invalid JWS - wrong number of parts
        let result = JwsManager::verify_jws("invalid.jws", &verifying_key);
        assert!(result.is_err());
        
        // Invalid JWS - empty
        let result = JwsManager::verify_jws("", &verifying_key);
        assert!(result.is_err());
    }
}