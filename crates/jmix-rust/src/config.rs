use serde::{Deserialize, Serialize};
use std::env;

/// Configuration for building JMIX envelopes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: Option<String>,
    pub sender: Entity,
    pub requester: Entity,
    pub receivers: Vec<Entity>,
    pub patient: PatientConfig,
    pub security: Option<SecurityConfig>,
    pub encryption: Option<EncryptionConfig>,
    pub custom_tags: Option<Vec<String>>,
    pub report: Option<ReportConfig>,
    pub files: Option<Vec<FileConfig>>,
    pub consent: Option<ConsentConfig>,
    pub deid_keys: Option<Vec<String>>,
    pub jws_signing_key: Option<String>,
    pub verify_assertions: Option<bool>,
}

/// Entity configuration (sender, requester, receiver)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub name: String,
    pub id: String,
    pub contact: ContactInfo,
    pub assertion: Option<AssertionConfig>,
}

/// Contact information for entities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContactInfo {
    Email(String),
    Detailed {
        system: String,
        value: String,
    },
}

/// Assertion configuration for cryptographic verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertionConfig {
    pub alg: String,
    pub public_key: String,
    pub fingerprint: Option<String>,
    pub key_reference: Option<String>,
    pub signature: Option<String>,
    pub expires_at: Option<String>,
    pub private_key: Option<String>, // For signing
}

/// Patient configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatientConfig {
    pub name: Option<String>,
    pub id: Option<String>,
    pub dob: Option<String>,
    pub sex: Option<String>,
    pub identifiers: Option<Vec<IdentifierConfig>>,
    pub verification: Option<VerificationConfig>,
}

/// Patient identifier configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifierConfig {
    pub system: String,
    pub value: String,
    pub use_field: Option<String>, // "use" is a Rust keyword
}

/// Patient verification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationConfig {
    pub status: String,
    pub method: String,
    pub timestamp: Option<String>,
}

/// Security configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub classification: Option<String>,
}

/// Encryption configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionConfig {
    pub recipient_public_key: String,
}

/// Report file configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportConfig {
    pub file: Option<String>,
    pub url: Option<String>,
}

/// Additional file configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FileConfig {
    Path(String),
    Detailed {
        path: String,
        name: Option<String>,
    },
}

/// Consent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentConfig {
    pub status: String,
    pub scope: Option<Vec<String>>,
    pub method: Option<String>,
}

/// Study configuration (for fallback when DICOM parsing fails)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyConfig {
    pub description: Option<String>,
    pub uid: Option<String>,
    pub modality: Option<String>,
    pub body_part: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: Some("1.0".to_string()),
            sender: Entity {
                name: "".to_string(),
                id: "".to_string(),
                contact: ContactInfo::Email("".to_string()),
                assertion: None,
            },
            requester: Entity {
                name: "".to_string(),
                id: "".to_string(),
                contact: ContactInfo::Email("".to_string()),
                assertion: None,
            },
            receivers: vec![],
            patient: PatientConfig {
                name: None,
                id: None,
                dob: None,
                sex: None,
                identifiers: None,
                verification: None,
            },
            security: Some(SecurityConfig {
                classification: Some("confidential".to_string()),
            }),
            encryption: None,
            custom_tags: None,
            report: None,
            files: None,
            consent: None,
            deid_keys: None,
            jws_signing_key: None,
            verify_assertions: None,
        }
    }
}

/// Configuration for validation (schema paths, etc.)
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    pub schema_path: String,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        let schema_path = env::var("JMIX_SCHEMA_DIR").unwrap_or_else(|_| "../jmix/schemas".to_string());
        Self { schema_path }
    }
}
