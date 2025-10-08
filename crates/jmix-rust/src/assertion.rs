//! JMIX Assertion Module
//!
//! This module handles sender and receiver assertions for identity verification 
//! and non-repudiation in JMIX envelopes according to the security specification.
//!
//! Features:
//! - Field-based signature generation
//! - Assertion creation and verification
//! - Directory attestation support
//! - Ed25519 signature verification

use crate::{
    jws::JwsManager,
    types::{Assertion, DirectoryAttestation, Entity, Manifest, SigningKey},
    error::{JmixError, JmixResult},
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{Map, Value};
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// Assertion manager for creating and verifying sender/receiver assertions
pub struct AssertionManager {
    /// JWS manager for signing operations
    jws_manager: JwsManager,
}

/// Result of assertion creation
#[derive(Debug, Clone)]
pub struct AssertionResult {
    /// The complete assertion ready for embedding
    pub assertion: Assertion,
    /// Fields that were signed (for verification)
    pub signed_data: String,
}

/// Verification result for an assertion
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// Assertion is valid
    Valid {
        /// Public key fingerprint
        fingerprint: String,
        /// Signed fields and values
        signed_fields: HashMap<String, Value>,
    },
    /// Assertion is invalid
    Invalid {
        /// Reason for invalidity
        reason: String,
    },
    /// Assertion expired
    Expired {
        /// Expiration timestamp
        expires_at: String,
    },
}

impl AssertionManager {
    /// Create a new assertion manager with a JWS manager
    pub fn new(jws_manager: JwsManager) -> Self {
        Self { jws_manager }
    }

    /// Create a new assertion manager with generated key
    pub fn with_generated_key() -> JmixResult<Self> {
        Ok(Self {
            jws_manager: JwsManager::with_generated_key()?,
        })
    }

    /// Create a new assertion manager with key from file
    pub fn with_key_file<P: AsRef<std::path::Path>>(key_path: P) -> JmixResult<Self> {
        Ok(Self {
            jws_manager: JwsManager::with_key_file(key_path)?,
        })
    }

    /// Get the public key information for this assertion manager
    pub fn public_key_info(&self) -> JmixResult<SigningKey> {
        let public_key_bytes = self.jws_manager.public_key_bytes();
        let public_key_b64 = BASE64.encode(&public_key_bytes);
        
        // Create fingerprint as SHA256 hash
        let mut hasher = Sha256::new();
        hasher.update(&public_key_bytes);
        let fingerprint = format!("SHA256:{:x}", hasher.finalize());

        Ok(SigningKey {
            alg: "Ed25519".to_string(),
            public_key: public_key_b64,
            fingerprint,
        })
    }

    /// Create a sender assertion for an entity
    pub fn create_sender_assertion(
        &self,
        entity: &Entity,
        manifest: &Manifest,
        expires_at: Option<String>,
        key_reference: Option<String>,
        directory_attestation: Option<DirectoryAttestation>,
    ) -> JmixResult<AssertionResult> {
        // Standard fields to sign for sender assertion
        let signed_fields = vec![
            "sender.id".to_string(),
            "sender.name".to_string(),
            "manifest.id".to_string(),
            "timestamp".to_string(),
        ];

        self.create_assertion(entity, manifest, signed_fields, expires_at, key_reference, directory_attestation)
    }

    /// Create a requester assertion for an entity  
    pub fn create_requester_assertion(
        &self,
        entity: &Entity,
        manifest: &Manifest,
        expires_at: Option<String>,
        key_reference: Option<String>,
        directory_attestation: Option<DirectoryAttestation>,
    ) -> JmixResult<AssertionResult> {
        // Standard fields to sign for requester assertion
        let signed_fields = vec![
            "requester.id".to_string(),
            "requester.name".to_string(),
            "manifest.id".to_string(),
            "timestamp".to_string(),
        ];

        self.create_assertion(entity, manifest, signed_fields, expires_at, key_reference, directory_attestation)
    }

    /// Create a receiver assertion for a specific receiver index
    pub fn create_receiver_assertion(
        &self,
        entity: &Entity,
        manifest: &Manifest,
        receiver_index: usize,
        expires_at: Option<String>,
        key_reference: Option<String>,
        directory_attestation: Option<DirectoryAttestation>,
    ) -> JmixResult<AssertionResult> {
        // Standard fields to sign for receiver assertion, bound to a specific index in manifest.receiver
        let signed_fields = vec![
            format!("receiver.{}.id", receiver_index),
            format!("receiver.{}.name", receiver_index),
            "manifest.id".to_string(),
            "timestamp".to_string(),
        ];

        self.create_assertion(entity, manifest, signed_fields, expires_at, key_reference, directory_attestation)
    }

    /// Create a custom assertion with specific fields
    pub fn create_custom_assertion(
        &self,
        entity: &Entity,
        manifest: &Manifest,
        signed_fields: Vec<String>,
        expires_at: Option<String>,
        key_reference: Option<String>,
        directory_attestation: Option<DirectoryAttestation>,
    ) -> JmixResult<AssertionResult> {
        self.create_assertion(entity, manifest, signed_fields, expires_at, key_reference, directory_attestation)
    }

    /// Internal method to create assertion
    fn create_assertion(
        &self,
        entity: &Entity,
        manifest: &Manifest,
        signed_fields: Vec<String>,
        expires_at: Option<String>,
        key_reference: Option<String>,
        directory_attestation: Option<DirectoryAttestation>,
    ) -> JmixResult<AssertionResult> {
        // Extract field values from manifest and entity
        let field_values = self.extract_field_values(entity, manifest, &signed_fields)?;
        
        // Canonicalize the field-value structure for signing
        let canonical_data = self.canonicalize_fields(&field_values)?;
        
        // Create signature
        let signature = self.jws_manager.sign_data(canonical_data.as_bytes())?;
        let signature_b64 = BASE64.encode(&signature);

        // Get public key info
        let signing_key = self.public_key_info()?;

        // Create assertion
        let assertion = Assertion {
            signing_key,
            key_reference,
            signed_fields,
            signature: signature_b64,
            expires_at,
            directory_attestation,
        };

        Ok(AssertionResult {
            assertion,
            signed_data: canonical_data,
        })
    }

    /// Extract field values from manifest and entity for signing
    fn extract_field_values(
        &self,
        entity: &Entity,
        manifest: &Manifest,
        signed_fields: &[String],
    ) -> JmixResult<HashMap<String, Value>> {
        let mut field_values = HashMap::new();

        // Convert manifest and entity to JSON for field extraction
        let manifest_json: Value = serde_json::to_value(manifest)?;
        let entity_json: Value = serde_json::to_value(entity)?;

        for field in signed_fields {
            let value = match field.as_str() {
                // Sender fields
                "sender.id" => manifest_json.pointer("/sender/id").cloned(),
                "sender.name" => manifest_json.pointer("/sender/name").cloned(),
                // Requester fields
                "requester.id" => manifest_json.pointer("/requester/id").cloned(),
                "requester.name" => manifest_json.pointer("/requester/name").cloned(),
                // Manifest fields
                "manifest.id" => Some(Value::String(manifest.id.clone())),
                "timestamp" => Some(Value::String(manifest.timestamp.clone())),
                "version" => Some(Value::String(manifest.version.clone())),
                // Security fields
                "security.payload_hash" => Some(Value::String(manifest.security.payload_hash.clone())),
                // Try to extract from entity directly
                _ if field.starts_with("entity.") => {
                    let field_path = field.strip_prefix("entity.").unwrap();
                    entity_json.pointer(&format!("/{}", field_path.replace('.', "/"))).cloned()
                },
                // Try to extract from manifest directly
                _ => {
                    let field_path = format!("/{}", field.replace('.', "/"));
                    manifest_json.pointer(&field_path).cloned()
                },
            };

            if let Some(val) = value {
                field_values.insert(field.clone(), val);
            } else {
                return Err(JmixError::Other(format!("Field '{}' not found for signing", field)));
            }
        }

        Ok(field_values)
    }

    /// Canonicalize field-value pairs for consistent signing
    fn canonicalize_fields(&self, field_values: &HashMap<String, Value>) -> JmixResult<String> {
        // Create a sorted map for deterministic serialization
        let mut sorted_map = Map::new();
        let mut keys: Vec<_> = field_values.keys().collect();
        keys.sort();

        for key in keys {
            if let Some(value) = field_values.get(key) {
                sorted_map.insert(key.clone(), value.clone());
            }
        }

        // Serialize to compact JSON (no pretty printing)
        let canonical = serde_json::to_string(&sorted_map)?;
        Ok(canonical)
    }

    /// Verify an assertion against a manifest and entity
    pub fn verify_assertion(
        assertion: &Assertion,
        entity: &Entity, 
        manifest: &Manifest,
    ) -> JmixResult<VerificationResult> {
        // Check expiration if present
        if let Some(expires_at) = &assertion.expires_at {
            let now = chrono::Utc::now();
            if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at) {
                if now > expiry {
                    return Ok(VerificationResult::Expired {
                        expires_at: expires_at.clone(),
                    });
                }
            }
        }

        // Extract public key
        let public_key_bytes = BASE64.decode(&assertion.signing_key.public_key)
            .map_err(|e| JmixError::Other(format!("Invalid public key: {}", e)))?;
        
        if public_key_bytes.len() != 32 {
            return Ok(VerificationResult::Invalid {
                reason: "Invalid public key length".to_string(),
            });
        }

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&public_key_bytes);
        let verify_key = VerifyingKey::from_bytes(&key_array)
            .map_err(|e| JmixError::Other(format!("Invalid Ed25519 key: {}", e)))?;

        // Create temporary assertion manager for field extraction
        let temp_manager = Self::with_generated_key()?; // We only need the extraction logic
        
        // Extract field values that should have been signed
        let field_values = temp_manager.extract_field_values(entity, manifest, &assertion.signed_fields)?;
        
        // Canonicalize the data
        let canonical_data = temp_manager.canonicalize_fields(&field_values)?;
        
        // Decode signature
        let signature_bytes = BASE64.decode(&assertion.signature)
            .map_err(|e| JmixError::Other(format!("Invalid signature: {}", e)))?;
        
        let signature = Signature::from_bytes(&signature_bytes.try_into()
            .map_err(|_| JmixError::Other("Invalid signature length".to_string()))?);

        // Verify signature
        match verify_key.verify(canonical_data.as_bytes(), &signature) {
            Ok(_) => {
                // Verify fingerprint matches
                let mut hasher = Sha256::new();
                hasher.update(&public_key_bytes);
                let expected_fingerprint = format!("SHA256:{:x}", hasher.finalize());
                
                if assertion.signing_key.fingerprint != expected_fingerprint {
                    return Ok(VerificationResult::Invalid {
                        reason: "Fingerprint mismatch".to_string(),
                    });
                }

                Ok(VerificationResult::Valid {
                    fingerprint: assertion.signing_key.fingerprint.clone(),
                    signed_fields: field_values,
                })
            },
            Err(_) => Ok(VerificationResult::Invalid {
                reason: "Signature verification failed".to_string(),
            }),
        }
    }

    /// Verify directory attestation (placeholder for future implementation)
    pub fn verify_directory_attestation(
        _attestation: &DirectoryAttestation,
    ) -> JmixResult<bool> {
        // TODO: Implement directory attestation verification
        // This would involve:
        // 1. Verifying the attestation_signature JWS
        // 2. Checking the attestation_public_key against known directory keys
        // 3. Validating the attestation covers the claimed identity
        Ok(true) // Placeholder - always pass for now
    }

    /// Get the JWS manager for access to signing capabilities
    pub fn jws_manager(&self) -> &JwsManager {
        &self.jws_manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Contact, Security};

    fn create_test_entity() -> Entity {
        Entity {
            name: Some("Test Clinic".to_string()),
            id: "org:test.clinic".to_string(),
            contact: Contact::Email("test@clinic.org".to_string()),
            assertion: None,
        }
    }

    fn create_test_manifest() -> Manifest {
        Manifest {
            version: "1.0".to_string(),
            id: "test-envelope-123".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            sender: create_test_entity(),
            requester: Some(create_test_entity()),
            receiver: vec![create_test_entity()],
            security: Security {
                classification: None,
                payload_hash: "sha256:abcd1234".to_string(),
                jws: None,
                signature: None,
                encryption: None,
            },
            extensions: None,
        }
    }

    #[test]
    fn test_assertion_manager_creation() -> JmixResult<()> {
        let manager = AssertionManager::with_generated_key()?;
        let public_key_info = manager.public_key_info()?;
        
        assert_eq!(public_key_info.alg, "Ed25519");
        assert!(!public_key_info.public_key.is_empty());
        assert!(public_key_info.fingerprint.starts_with("SHA256:"));
        
        Ok(())
    }

    #[test]
    fn test_sender_assertion_creation() -> JmixResult<()> {
        let manager = AssertionManager::with_generated_key()?;
        let entity = create_test_entity();
        let manifest = create_test_manifest();

        let result = manager.create_sender_assertion(
            &entity,
            &manifest,
            Some("2025-01-01T00:00:00Z".to_string()),
            Some("test://key-ref".to_string()),
            None,
        )?;

        assert_eq!(result.assertion.signing_key.alg, "Ed25519");
        assert!(!result.assertion.signature.is_empty());
        assert!(result.assertion.signed_fields.contains(&"sender.id".to_string()));
        assert!(result.assertion.signed_fields.contains(&"sender.name".to_string()));
        assert_eq!(result.assertion.expires_at, Some("2025-01-01T00:00:00Z".to_string()));

        Ok(())
    }

    #[test]
    fn test_assertion_verification() -> JmixResult<()> {
        let manager = AssertionManager::with_generated_key()?;
        let entity = create_test_entity();
        let manifest = create_test_manifest();

        // Create assertion
        let result = manager.create_sender_assertion(
            &entity,
            &manifest,
            None,
            None,
            None,
        )?;

        // Verify assertion
        let verification = AssertionManager::verify_assertion(
            &result.assertion,
            &entity,
            &manifest,
        )?;

        match verification {
            VerificationResult::Valid { fingerprint, signed_fields } => {
                assert_eq!(fingerprint, result.assertion.signing_key.fingerprint);
                assert!(!signed_fields.is_empty());
            },
            _ => panic!("Expected valid assertion"),
        }

        Ok(())
    }

    #[test]
    fn test_receiver_assertion_creation_and_verification() -> JmixResult<()> {
        let manager = AssertionManager::with_generated_key()?;
        let mut manifest = create_test_manifest();
        // Ensure there is at least one receiver with Some(name)
        if manifest.receiver.is_empty() {
            manifest.receiver.push(create_test_entity());
        }
        let idx = 0usize;
        let receiver = manifest.receiver[idx].clone();

        let result = manager.create_receiver_assertion(
            &receiver,
            &manifest,
            idx,
            None,
            None,
            None,
        )?;

        assert!(!result.assertion.signature.is_empty());
        assert!(result.assertion.signed_fields.iter().any(|f| f == &format!("receiver.{}.id", idx)));

        let verification = AssertionManager::verify_assertion(
            &result.assertion,
            &manifest.receiver[idx],
            &manifest,
        )?;
        match verification {
            VerificationResult::Valid { .. } => {},
            _ => panic!("Expected valid receiver assertion"),
        }
        Ok(())
    }

    #[test]
    fn test_invalid_assertion_verification() -> JmixResult<()> {
        let manager1 = AssertionManager::with_generated_key()?;
        let manager2 = AssertionManager::with_generated_key()?;
        let entity = create_test_entity();
        let manifest = create_test_manifest();

        // Create assertion with one key
        let result = manager1.create_sender_assertion(&entity, &manifest, None, None, None)?;
        
        // Modify the assertion to use different public key (simulating tampering)
        let mut tampered_assertion = result.assertion.clone();
        let other_public_key = manager2.public_key_info()?;
        tampered_assertion.signing_key = other_public_key;

        // Verify should fail
        let verification = AssertionManager::verify_assertion(
            &tampered_assertion,
            &entity,
            &manifest,
        )?;

        match verification {
            VerificationResult::Invalid { reason: _ } => {
                // Expected
            },
            _ => panic!("Expected invalid assertion"),
        }

        Ok(())
    }

    #[test] 
    fn test_expired_assertion() -> JmixResult<()> {
        let manager = AssertionManager::with_generated_key()?;
        let entity = create_test_entity();
        let manifest = create_test_manifest();

        // Create expired assertion
        let result = manager.create_sender_assertion(
            &entity,
            &manifest,
            Some("2020-01-01T00:00:00Z".to_string()), // Past date
            None,
            None,
        )?;

        let verification = AssertionManager::verify_assertion(
            &result.assertion,
            &entity,
            &manifest,
        )?;

        match verification {
            VerificationResult::Expired { expires_at: _ } => {
                // Expected
            },
            _ => panic!("Expected expired assertion"),
        }

        Ok(())
    }
}