use thiserror::Error;

/// Main error type for JMIX operations
#[derive(Error, Debug)]
pub enum JmixError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),

    #[error("Cryptography error: {0}")]
    Cryptography(#[from] CryptographyError),
    
    #[error("Encryption error: {0}")]
    Encryption(#[from] crate::encryption::EncryptionError),

    #[error("DICOM processing error: {0}")]
    Dicom(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("JMIX error: {0}")]
    General(String),
    
    #[error("Other error: {0}")]
    Other(String),
}

/// Validation-specific errors
#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("Schema file not found: {path}")]
    SchemaNotFound { path: String },

    #[error("Schema validation failed for {schema}: {errors:?}")]
    SchemaValidation {
        schema: String,
        errors: Vec<String>,
    },

    #[error("Invalid JSON schema: {0}")]
    InvalidSchema(String),
}

/// Cryptography-specific errors
#[derive(Error, Debug)]
pub enum CryptographyError {
    #[error("Key generation failed: {0}")]
    KeyGeneration(String),

    #[error("Encryption failed: {0}")]
    Encryption(String),

    #[error("Decryption failed: {0}")]
    Decryption(String),

    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    #[error("JWS creation failed: {0}")]
    JwsCreation(String),

    #[error("JWS verification failed: {0}")]
    JwsVerification(String),

    #[error("Hash verification failed")]
    HashVerification,
}

/// Convenience result type
pub type JmixResult<T> = Result<T, JmixError>;