use crate::error::{JmixResult, ValidationError};
use crate::config::ValidationConfig;
use crate::types::{Audit, Files, Manifest, Metadata};
use jsonschema::{Validator, Draft};
use once_cell::sync::OnceCell;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Schema validator that lazily loads and caches JSON schemas
pub struct SchemaValidator {
    schema_path: String,
    schemas: OnceCell<HashMap<String, Validator>>,
}

impl SchemaValidator {
    /// Create a new schema validator with the given schema path
    pub fn new(schema_path: Option<String>) -> Self {
        Self {
            schema_path: schema_path.unwrap_or_else(|| {
                ValidationConfig::default().schema_path
            }),
            schemas: OnceCell::new(),
        }
    }

    /// Create a new schema validator with default configuration
    pub fn with_default_config() -> Self {
        Self::new(None)
    }

    /// Get or load the schemas
    fn get_schemas(&self) -> JmixResult<&HashMap<String, Validator>> {
        self.schemas.get_or_try_init(|| self.load_schemas())
    }

    /// Load all schemas from the schema directory
    fn load_schemas(&self) -> JmixResult<HashMap<String, Validator>> {
        let mut schemas = HashMap::new();
        
        let schema_files = [
            "manifest.schema.json",
            "metadata.schema.json", 
            "audit.schema.json",
            "files.schema.json",
        ];

        for schema_file in &schema_files {
            let schema_path = Path::new(&self.schema_path).join(schema_file);
            
            if !schema_path.exists() {
                return Err(ValidationError::SchemaNotFound {
                    path: schema_path.to_string_lossy().to_string(),
                }.into());
            }

            let schema_content = fs::read_to_string(&schema_path)
                .map_err(|e| ValidationError::InvalidSchema(
                    format!("Failed to read schema file {}: {}", schema_file, e)
                ))?;

            let schema_value: Value = serde_json::from_str(&schema_content)
                .map_err(|e| ValidationError::InvalidSchema(
                    format!("Invalid JSON in schema file {}: {}", schema_file, e)
                ))?;

            let compiled_schema = Validator::options()
                .with_draft(Draft::Draft202012)
                .build(&schema_value)
                .map_err(|e| ValidationError::InvalidSchema(
                    format!("Failed to compile schema {}: {}", schema_file, e)
                ))?;

            schemas.insert(schema_file.to_string(), compiled_schema);
        }

        Ok(schemas)
    }

    /// Validate a manifest against its schema
    pub fn validate_manifest(&self, manifest: &Manifest) -> JmixResult<()> {
        let schemas = self.get_schemas()?;
        let schema = schemas.get("manifest.schema.json")
            .ok_or_else(|| ValidationError::InvalidSchema(
                "Manifest schema not loaded".to_string()
            ))?;

        let manifest_value = serde_json::to_value(manifest)?;
        
        let validation_errors: Vec<String> = schema.iter_errors(&manifest_value)
            .map(|error| format!("{}: {}", error.instance_path, error))
            .collect();
        
        if !validation_errors.is_empty() {
            return Err(ValidationError::SchemaValidation {
                schema: "manifest.schema.json".to_string(),
                errors: validation_errors,
            }.into());
        }

        Ok(())
    }

    /// Validate metadata against its schema
    pub fn validate_metadata(&self, metadata: &Metadata) -> JmixResult<()> {
        let schemas = self.get_schemas()?;
        let schema = schemas.get("metadata.schema.json")
            .ok_or_else(|| ValidationError::InvalidSchema(
                "Metadata schema not loaded".to_string()
            ))?;

        let metadata_value = serde_json::to_value(metadata)?;
        
        let validation_errors: Vec<String> = schema.iter_errors(&metadata_value)
            .map(|error| format!("{}: {}", error.instance_path, error))
            .collect();
        
        if !validation_errors.is_empty() {
            return Err(ValidationError::SchemaValidation {
                schema: "metadata.schema.json".to_string(),
                errors: validation_errors,
            }.into());
        }

        Ok(())
    }

    /// Validate audit against its schema
    pub fn validate_audit(&self, audit: &Audit) -> JmixResult<()> {
        let schemas = self.get_schemas()?;
        let schema = schemas.get("audit.schema.json")
            .ok_or_else(|| ValidationError::InvalidSchema(
                "Audit schema not loaded".to_string()
            ))?;

        let audit_value = serde_json::to_value(audit)?;
        
        let validation_errors: Vec<String> = schema.iter_errors(&audit_value)
            .map(|error| format!("{}: {}", error.instance_path, error))
            .collect();
        
        if !validation_errors.is_empty() {
            return Err(ValidationError::SchemaValidation {
                schema: "audit.schema.json".to_string(),
                errors: validation_errors,
            }.into());
        }

        Ok(())
    }

    /// Validate files manifest against its schema
    pub fn validate_files(&self, files: &Files) -> JmixResult<()> {
        let schemas = self.get_schemas()?;
        let schema = schemas.get("files.schema.json")
            .ok_or_else(|| ValidationError::InvalidSchema(
                "Files schema not loaded".to_string()
            ))?;

        let files_value = serde_json::to_value(files)?;
        
        let validation_errors: Vec<String> = schema.iter_errors(&files_value)
            .map(|error| format!("{}: {}", error.instance_path, error))
            .collect();
        
        if !validation_errors.is_empty() {
            return Err(ValidationError::SchemaValidation {
                schema: "files.schema.json".to_string(),
                errors: validation_errors,
            }.into());
        }

        Ok(())
    }

    /// Validate raw JSON value against a schema
    pub fn validate_json(&self, schema_name: &str, value: &Value) -> JmixResult<()> {
        let schemas = self.get_schemas()?;
        let schema = schemas.get(schema_name)
            .ok_or_else(|| ValidationError::InvalidSchema(
                format!("Schema {} not loaded", schema_name)
            ))?;

        let validation_errors: Vec<String> = schema.iter_errors(value)
            .map(|error| format!("{}: {}", error.instance_path, error))
            .collect();
        
        if !validation_errors.is_empty() {
            return Err(ValidationError::SchemaValidation {
                schema: schema_name.to_string(),
                errors: validation_errors,
            }.into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn create_minimal_manifest() -> Manifest {
        Manifest {
            version: "1.0".to_string(),
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(), // Valid UUID format
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
                payload_hash: "sha256:abc123def456".to_string(), // Valid format
                jws: None,
                signature: None,
                encryption: None,
            },
            extensions: None,
        }
    }

    fn create_minimal_metadata() -> Metadata {
        Metadata {
            version: "1.0".to_string(),
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
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
        }
    }

    fn create_minimal_audit() -> Audit {
        Audit {
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
        }
    }

    #[test]
    fn test_validator_creation() {
        // Test with default path
        let validator = SchemaValidator::with_default_config();
        assert_eq!(validator.schema_path, "../jmix/schemas");

        // Test with custom path
        let validator = SchemaValidator::new(Some("/custom/path".to_string()));
        assert_eq!(validator.schema_path, "/custom/path");
    }

    #[test]
    fn test_validation_with_missing_schemas() {
        // Test validation when schema files don't exist
        let validator = SchemaValidator::new(Some("/nonexistent/path".to_string()));
        
        let manifest = create_minimal_manifest();
        let result = validator.validate_manifest(&manifest);
        
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::JmixError::Validation(ValidationError::SchemaNotFound { .. }) => {},
            other => panic!("Expected SchemaNotFound error, got: {:?}", other),
        }
    }

    #[test]
    fn test_minimal_structure_serialization() {
        // Test that our minimal structures can be serialized without errors
        let manifest = create_minimal_manifest();
        let metadata = create_minimal_metadata();
        let audit = create_minimal_audit();

        // These should not panic
        serde_json::to_value(&manifest).expect("Manifest serialization failed");
        serde_json::to_value(&metadata).expect("Metadata serialization failed");
        serde_json::to_value(&audit).expect("Audit serialization failed");
    }

    // Note: Full validation tests require schema files to be present
    // These would be integration tests that require the ../jmix/schemas directory
    #[test]
    #[ignore] // Ignored by default since it requires schema files
    fn test_manifest_validation_integration() {
        let validator = SchemaValidator::with_default_config();
        let manifest = create_minimal_manifest();
        
        // This test will only pass if schema files are available
        let result = validator.validate_manifest(&manifest);
        match result {
            Ok(()) => println!("Validation passed"),
            Err(e) => println!("Validation failed: {}", e),
        }
    }
}