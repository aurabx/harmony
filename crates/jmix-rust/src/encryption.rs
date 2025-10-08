//! AES-256-GCM encryption with ECDH key agreement for JMIX envelopes.
//!
//! This module implements the JMIX encryption specification using:
//! - ECDH key agreement over Curve25519
//! - HKDF key derivation with SHA-256
//! - AES-256-GCM for authenticated encryption

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce, Key
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use std::{fs, io::Write, path::Path};
use thiserror::Error;
use x25519_dalek::{EphemeralSecret, PublicKey, x25519};
use zeroize::Zeroizing;
use crate::types::EncryptionInfo;

/// Errors that can occur during encryption/decryption operations
#[derive(Debug, Error)]
pub enum EncryptionError {
    #[error("Failed to generate random bytes: {0}")]
    RandomGeneration(String),
    
    #[error("Failed to encrypt data: {0}")]
    EncryptionFailed(String),
    
    #[error("Failed to decrypt data: {0}")]
    DecryptionFailed(String),
    
    #[error("Invalid key format: {0}")]
    InvalidKey(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Base64 decode error: {0}")]
    Base64Decode(#[from] base64::DecodeError),
    
    #[error("Invalid nonce size: expected 12 bytes, got {0}")]
    InvalidNonceSize(usize),
    
    #[error("Invalid auth tag size: expected 16 bytes, got {0}")]
    InvalidAuthTagSize(usize),
}


/// Manager for JMIX envelope encryption using AES-256-GCM with ECDH
pub struct EncryptionManager {
    /// The recipient's long-term public key
    recipient_public_key: PublicKey,
}

/// Result of encryption operation
#[derive(Debug)]
pub struct EncryptionResult {
    /// Encrypted data
    pub ciphertext: Vec<u8>,
    /// Encryption metadata for the manifest
    pub info: EncryptionInfo,
}

/// Key pair for JMIX encryption (Curve25519)
/// The secret key is automatically zeroed when dropped for security
pub struct KeyPair {
    /// Secret key (32 bytes) - automatically zeroed on drop
    pub secret: Zeroizing<[u8; 32]>,
    /// Public key (32 bytes) 
    pub public: PublicKey,
}

impl EncryptionManager {
    /// Create a new encryption manager with the recipient's public key
    pub fn new(recipient_public_key: PublicKey) -> Self {
        Self {
            recipient_public_key,
        }
    }
    
    /// Create an encryption manager from a base64-encoded recipient public key
    pub fn from_base64_public_key(public_key_b64: &str) -> Result<Self, EncryptionError> {
        let key_bytes = BASE64.decode(public_key_b64)?;
        if key_bytes.len() != 32 {
            return Err(EncryptionError::InvalidKey(
                format!("Expected 32 bytes, got {}", key_bytes.len())
            ));
        }
        
        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);
        let public_key = PublicKey::from(key_array);
        
        Ok(Self::new(public_key))
    }
    
    /// Create an encryption manager by loading a public key from file
    pub fn from_public_key_file<P: AsRef<Path>>(path: P) -> Result<Self, EncryptionError> {
        let key_bytes = fs::read(path)?;
        if key_bytes.len() != 32 {
            return Err(EncryptionError::InvalidKey(
                format!("Expected 32 bytes in key file, got {}", key_bytes.len())
            ));
        }
        
        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);
        let public_key = PublicKey::from(key_array);
        
        Ok(Self::new(public_key))
    }
    
    /// Encrypt data using AES-256-GCM with ephemeral ECDH key agreement
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptionResult, EncryptionError> {
        // Generate ephemeral keypair for this encryption
        let ephemeral_secret = EphemeralSecret::random_from_rng(OsRng);
        let ephemeral_public = PublicKey::from(&ephemeral_secret);
        
        // Perform ECDH key agreement
        let shared_secret = ephemeral_secret.diffie_hellman(&self.recipient_public_key);
        
        // Derive symmetric key using HKDF-SHA256
        let hkdf = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
        let mut symmetric_key = [0u8; 32]; // AES-256 key
        hkdf.expand(b"JMIX-AES256-GCM", &mut symmetric_key)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("HKDF expansion failed: {}", e)))?;
        
        // Generate random IV (12 bytes for GCM)
        let mut iv = [0u8; 12];
        OsRng.fill_bytes(&mut iv);
        
        // Encrypt with AES-256-GCM
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&symmetric_key));
        let nonce = Nonce::from_slice(&iv);
        
        let ciphertext = cipher.encrypt(nonce, plaintext)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("AES-GCM encryption failed: {}", e)))?;
        
        // Split ciphertext and auth tag (last 16 bytes)
        if ciphertext.len() < 16 {
            return Err(EncryptionError::EncryptionFailed("Ciphertext too short".to_string()));
        }
        
        let (data, auth_tag) = ciphertext.split_at(ciphertext.len() - 16);
        
        // Create encryption info
        let info = EncryptionInfo {
            algorithm: "AES-256-GCM".to_string(),
            ephemeral_public_key: BASE64.encode(ephemeral_public.as_bytes()),
            iv: BASE64.encode(&iv),
            auth_tag: BASE64.encode(auth_tag),
        };
        
        Ok(EncryptionResult {
            ciphertext: data.to_vec(),
            info,
        })
    }
}

/// Decryption manager for JMIX envelopes
/// The secret key is automatically zeroed when dropped for security
pub struct DecryptionManager {
    /// The recipient's long-term secret key - automatically zeroed on drop
    secret_key: Zeroizing<[u8; 32]>,
}

impl DecryptionManager {
    /// Create a new decryption manager with the recipient's secret key
    pub fn new(secret_key: [u8; 32]) -> Self {
        Self { secret_key: Zeroizing::new(secret_key) }
    }
    
    /// Create a decryption manager from raw secret key bytes
    pub fn from_bytes(key_bytes: [u8; 32]) -> Self {
        Self::new(key_bytes)
    }
    
    /// Create a decryption manager by loading a secret key from file
    pub fn from_secret_key_file<P: AsRef<Path>>(path: P) -> Result<Self, EncryptionError> {
        let key_bytes = fs::read(path)?;
        if key_bytes.len() != 32 {
            return Err(EncryptionError::InvalidKey(
                format!("Expected 32 bytes in key file, got {}", key_bytes.len())
            ));
        }
        
        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);
        
        Ok(Self::from_bytes(key_array))
    }
    
    /// Decrypt data using the encryption info from the manifest
    pub fn decrypt(&self, ciphertext: &[u8], info: &EncryptionInfo) -> Result<Vec<u8>, EncryptionError> {
        // Validate algorithm
        if info.algorithm != "AES-256-GCM" {
            return Err(EncryptionError::DecryptionFailed(
                format!("Unsupported algorithm: {}", info.algorithm)
            ));
        }
        
        // Decode the ephemeral public key
        let ephemeral_public_bytes = BASE64.decode(&info.ephemeral_public_key)?;
        if ephemeral_public_bytes.len() != 32 {
            return Err(EncryptionError::InvalidKey(
                format!("Invalid ephemeral public key length: {}", ephemeral_public_bytes.len())
            ));
        }
        
        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&ephemeral_public_bytes);
        let ephemeral_public = PublicKey::from(key_array);
        
        // Decode IV and auth tag
        let iv_bytes = BASE64.decode(&info.iv)?;
        let auth_tag_bytes = BASE64.decode(&info.auth_tag)?;
        
        if iv_bytes.len() != 12 {
            return Err(EncryptionError::InvalidNonceSize(iv_bytes.len()));
        }
        
        if auth_tag_bytes.len() != 16 {
            return Err(EncryptionError::InvalidAuthTagSize(auth_tag_bytes.len()));
        }
        
        // Perform ECDH key agreement
        // Use raw x25519 function: shared_secret = our_secret * their_public
        let shared_secret_bytes = x25519(*self.secret_key, ephemeral_public.to_bytes());
        
        // Derive symmetric key using HKDF-SHA256
        let hkdf = Hkdf::<Sha256>::new(None, &shared_secret_bytes);
        let mut symmetric_key = [0u8; 32];
        hkdf.expand(b"JMIX-AES256-GCM", &mut symmetric_key)
            .map_err(|e| EncryptionError::DecryptionFailed(format!("HKDF expansion failed: {}", e)))?;
        
        // Reconstruct the full ciphertext with auth tag
        let mut full_ciphertext = ciphertext.to_vec();
        full_ciphertext.extend_from_slice(&auth_tag_bytes);
        
        // Decrypt with AES-256-GCM
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&symmetric_key));
        let nonce = Nonce::from_slice(&iv_bytes);
        
        let plaintext = cipher.decrypt(nonce, full_ciphertext.as_slice())
            .map_err(|e| EncryptionError::DecryptionFailed(format!("AES-GCM decryption failed: {}", e)))?;
        
        Ok(plaintext)
    }
}

impl KeyPair {
    /// Generate a new random keypair for encryption
    pub fn generate() -> Self {
        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        
        // Note: We use manual base point multiplication here because x25519-dalek 2.0
        // doesn't expose secret key bytes from EphemeralSecret. In production code,
        // consider using a StaticSecret equivalent or a different key management approach.
        // The base point for X25519 is 9 (little-endian encoding)
        let mut base_point = [0u8; 32];
        base_point[0] = 9;
        let public_bytes = x25519(secret, base_point);
        let public = PublicKey::from(public_bytes);
        
        Self { 
            secret: Zeroizing::new(secret), 
            public 
        }
    }
    
    /// Create a keypair from raw secret key bytes
    pub fn from_secret_bytes(secret_bytes: [u8; 32]) -> Self {
        // Derive public key from secret key using x25519 base point multiplication
        // Note: This performs the same operation as the well-tested x25519-dalek library
        // The base point for X25519 is 9 (little-endian encoding)
        let mut base_point = [0u8; 32];
        base_point[0] = 9;
        let public_bytes = x25519(secret_bytes, base_point);
        let public = PublicKey::from(public_bytes);
        Self { 
            secret: Zeroizing::new(secret_bytes), 
            public 
        }
    }
    
    /// Get the secret key as bytes
    pub fn secret_bytes(&self) -> [u8; 32] {
        *self.secret
    }
    
    /// Get the public key as bytes
    pub fn public_bytes(&self) -> [u8; 32] {
        self.public.to_bytes()
    }
    
    /// Get the public key as base64 string
    pub fn public_key_base64(&self) -> String {
        BASE64.encode(self.public.as_bytes())
    }
    
    /// Save the keypair to files (secret key and public key)
    pub fn save_to_files<P: AsRef<Path>>(&self, secret_path: P, public_path: P) -> Result<(), EncryptionError> {
        // Save secret key
        let mut secret_file = fs::File::create(secret_path)?;
        secret_file.write_all(&self.secret_bytes())?;
        
        // Save public key
        let mut public_file = fs::File::create(public_path)?;
        public_file.write_all(&self.public_bytes())?;
        
        Ok(())
    }
    
    /// Load a keypair from a secret key file (derives public key)
    pub fn load_from_secret_file<P: AsRef<Path>>(secret_path: P) -> Result<Self, EncryptionError> {
        let secret_bytes = fs::read(secret_path)?;
        if secret_bytes.len() != 32 {
            return Err(EncryptionError::InvalidKey(
                format!("Expected 32 bytes in secret key file, got {}", secret_bytes.len())
            ));
        }
        
        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&secret_bytes);
        
        Ok(Self::from_secret_bytes(key_array))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_keypair_generation() {
        let keypair = KeyPair::generate();
        
        // Keys should be 32 bytes each
        assert_eq!(keypair.secret_bytes().len(), 32);
        assert_eq!(keypair.public_bytes().len(), 32);
        
        // Base64 encoding should work
        let public_b64 = keypair.public_key_base64();
        assert!(!public_b64.is_empty());
    }
    
    #[test]
    fn test_keypair_save_load() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let secret_path = temp_dir.path().join("secret.key");
        let public_path = temp_dir.path().join("public.key");
        
        // Generate and save keypair
        let original_keypair = KeyPair::generate();
        original_keypair.save_to_files(&secret_path, &public_path)?;
        
        // Load keypair back
        let loaded_keypair = KeyPair::load_from_secret_file(&secret_path)?;
        
        // Should be identical
        assert_eq!(original_keypair.secret_bytes(), loaded_keypair.secret_bytes());
        assert_eq!(original_keypair.public_bytes(), loaded_keypair.public_bytes());
        
        Ok(())
    }
    
    #[test]
    fn test_encryption_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        // Generate recipient keypair
        let recipient_keypair = KeyPair::generate();
        
        // Create encryption and decryption managers
        let encryption_manager = EncryptionManager::new(recipient_keypair.public);
        let decryption_manager = DecryptionManager::new(*recipient_keypair.secret);
        
        // Test data
        let plaintext = b"Hello, JMIX encryption!";
        
        // Encrypt
        let result = encryption_manager.encrypt(plaintext)?;
        assert!(result.ciphertext.len() > 0);
        assert_eq!(result.info.algorithm, "AES-256-GCM");
        
        // Decrypt
        let decrypted = decryption_manager.decrypt(&result.ciphertext, &result.info)?;
        assert_eq!(decrypted, plaintext);
        
        Ok(())
    }
    
    #[test]
    fn test_encryption_manager_from_base64() -> Result<(), Box<dyn std::error::Error>> {
        let keypair = KeyPair::generate();
        let public_b64 = keypair.public_key_base64();
        
        let manager = EncryptionManager::from_base64_public_key(&public_b64)?;
        
        // Should be able to encrypt
        let plaintext = b"Test message";
        let result = manager.encrypt(plaintext)?;
        assert!(result.ciphertext.len() > 0);
        
        Ok(())
    }
    
    #[test]
    fn test_encryption_different_ephemeral_keys() -> Result<(), Box<dyn std::error::Error>> {
        let recipient_keypair = KeyPair::generate();
        let encryption_manager = EncryptionManager::new(recipient_keypair.public);
        
        let plaintext = b"Same message";
        
        // Encrypt twice
        let result1 = encryption_manager.encrypt(plaintext)?;
        let result2 = encryption_manager.encrypt(plaintext)?;
        
        // Should have different ephemeral keys and IVs
        assert_ne!(result1.info.ephemeral_public_key, result2.info.ephemeral_public_key);
        assert_ne!(result1.info.iv, result2.info.iv);
        assert_ne!(result1.ciphertext, result2.ciphertext);
        
        Ok(())
    }
    
    #[test]
    fn test_invalid_decryption() -> Result<(), Box<dyn std::error::Error>> {
        let recipient_keypair = KeyPair::generate();
        let wrong_keypair = KeyPair::generate(); // Different keypair
        
        let encryption_manager = EncryptionManager::new(recipient_keypair.public);
        let wrong_decryption_manager = DecryptionManager::new(*wrong_keypair.secret);
        
        let plaintext = b"Secret message";
        let result = encryption_manager.encrypt(plaintext)?;
        
        // Should fail to decrypt with wrong key
        let decrypt_result = wrong_decryption_manager.decrypt(&result.ciphertext, &result.info);
        assert!(decrypt_result.is_err());
        
        Ok(())
    }
}