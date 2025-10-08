use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Complete JMIX envelope containing all components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub manifest: Manifest,
    pub metadata: Metadata,
    pub audit: Audit,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_jws: Option<String>, // JWS content if present
}

/// JMIX manifest.json structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub id: String,
    pub timestamp: String,
    pub sender: Entity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Entity>,
    pub receiver: Vec<Entity>,
    pub security: Security,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Extensions>,
}

/// Entity information (sender, requester, receiver)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub name: Option<String>,
    pub id: String,
    pub contact: Contact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertion: Option<Assertion>,
}

/// Contact information - can be string (email) or structured
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Contact {
    Email(String),
    Point(ContactPoint),
}

/// Structured contact information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactPoint {
    pub system: String,
    pub value: String,
}

/// Cryptographic assertion for entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assertion {
    pub signing_key: SigningKey,
    pub key_reference: Option<String>,
    pub signed_fields: Vec<String>,
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory_attestation: Option<DirectoryAttestation>,
}

/// Signing key information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningKey {
    pub alg: String, // \"Ed25519\"
    pub public_key: String,
    pub fingerprint: String, // SHA256:...
}

/// Directory attestation for PKI verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryAttestation {
    pub provider: String,
    pub attestation_signature: String,
    pub attestation_timestamp: String,
    pub attestation_public_key: String,
}

/// Security information in manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Security {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    pub payload_hash: String, // sha256:...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jws: Option<JwsReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<LegacySignature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionInfo>,
}

/// JWS file reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwsReference {
    pub jws_file: String,
}

/// Legacy signature format (for backward compatibility)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacySignature {
    pub alg: String,
    pub sig: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// Encryption information in manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionInfo {
    pub algorithm: String, // \"AES-256-GCM\"
    pub ephemeral_public_key: String,
    pub iv: String,
    pub auth_tag: String,
}

/// Extensions object for custom data
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Extensions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent: Option<ConsentExtension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deid: Option<DeidExtension>,
    #[serde(flatten)]
    pub additional: HashMap<String, Value>,
}

/// Consent extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentExtension {
    pub status: String, // \"granted\", \"denied\", \"unknown\"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_on: Option<String>,
}

/// De-identification extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeidExtension {
    pub keys: Vec<String>,
}

/// JMIX metadata.json structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub version: String,
    pub id: String,
    pub timestamp: String,
    pub patient: Patient,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Report>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub studies: Option<Studies>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Extensions>,
}

/// Patient information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patient {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<HumanName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dob: Option<String>, // YYYY-MM-DD
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sex: Option<String>, // \"M\", \"F\", \"O\"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifiers: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<Verification>,
}

/// Human name structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Identifier for patients or entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identifier {
    pub system: String,
    pub value: String,
}

/// Verification information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_on: Option<String>,
}

/// Report reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Studies information from DICOM
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Studies {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<Series>>,
}

/// Series information from DICOM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    pub series_uid: String,
    pub modality: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_part: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_count: Option<i32>,
}

/// JMIX audit.json structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Audit {
    pub audit: Vec<AuditEntry>,
}

/// Single audit entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub event: String,
    pub by: EntityRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<EntityRef>,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertion: Option<Assertion>,
}

/// Entity reference in audit entries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRef {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// JMIX files.json structure (array of file entries)
pub type Files = Vec<FileEntry>;

/// Single file entry in files.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_serialization_roundtrip() {
        let envelope = Envelope {
            manifest: Manifest {
                version: "1.0".to_string(),
                id: "test-id".to_string(),
                timestamp: "2023-01-01T00:00:00Z".to_string(),
                sender: Entity {
                    name: Some("Test Sender".to_string()),
                    id: "sender-1".to_string(),
                    contact: Contact::Email("sender@example.com".to_string()),
                    assertion: None,
                },
                requester: None,
                receiver: vec![Entity {
                    name: Some("Test Receiver".to_string()),
                    id: "receiver-1".to_string(),
                    contact: Contact::Email("receiver@example.com".to_string()),
                    assertion: None,
                }],
                security: Security {
                    classification: Some("confidential".to_string()),
                    payload_hash: "sha256:abc123".to_string(),
                    jws: None,
                    signature: None,
                    encryption: None,
                },
                extensions: None,
            },
            metadata: Metadata {
                version: "1.0".to_string(),
                id: "test-id".to_string(),
                timestamp: "2023-01-01T00:00:00Z".to_string(),
                patient: Patient {
                    id: "patient-1".to_string(),
                    name: None,
                    dob: None,
                    sex: None,
                    identifiers: None,
                    verification: None,
                },
                report: None,
                studies: None,
                extensions: None,
            },
            audit: Audit {
                audit: vec![AuditEntry {
                    event: "created".to_string(),
                    by: EntityRef {
                        id: "sender-1".to_string(),
                        name: Some("Test Sender".to_string()),
                    },
                    to: None,
                    timestamp: "2023-01-01T00:00:00Z".to_string(),
                    assertion: None,
                }],
            },
            manifest_jws: None,
        };

        // Test serialization to JSON
        let json = serde_json::to_string_pretty(&envelope).expect("Failed to serialize");
        assert!(!json.is_empty());

        // Test deserialization from JSON
        let deserialized: Envelope = serde_json::from_str(&json).expect("Failed to deserialize");
        
        // Basic validation that key fields match
        assert_eq!(envelope.manifest.id, deserialized.manifest.id);
        assert_eq!(envelope.metadata.patient.id, deserialized.metadata.patient.id);
        assert_eq!(envelope.audit.audit.len(), deserialized.audit.audit.len());
    }

    #[test]
    fn test_contact_variants() {
        // Test email contact
        let email_contact = Contact::Email("test@example.com".to_string());
        let json = serde_json::to_string(&email_contact).expect("Failed to serialize");
        let deserialized: Contact = serde_json::from_str(&json).expect("Failed to deserialize");
        matches!(deserialized, Contact::Email(_));

        // Test structured contact
        let point_contact = Contact::Point(ContactPoint {
            system: "phone".to_string(),
            value: "+1234567890".to_string(),
        });
        let json = serde_json::to_string(&point_contact).expect("Failed to serialize");
        let deserialized: Contact = serde_json::from_str(&json).expect("Failed to deserialize");
        matches!(deserialized, Contact::Point(_));
    }
}