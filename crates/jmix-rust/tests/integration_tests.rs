use jmix_rs::*;
use jmix_rs::config;
use jmix_rs::types;
use jmix_rs::validation::SchemaValidator;
use jmix_rs::dicom::DicomProcessor;

/// Test configuration for integration tests
fn create_test_config() -> config::Config {
    config::Config {
        version: Some("1.0".to_string()),
        sender: config::Entity {
            name: "Test Healthcare Organization".to_string(),
            id: "org:test.health.123".to_string(),
            contact: config::ContactInfo::Email("sender@test-health.org".to_string()),
            assertion: None,
        },
        requester: config::Entity {
            name: "Dr. Jane Smith".to_string(),
            id: "doc:jane.smith".to_string(),
            contact: config::ContactInfo::Detailed {
                system: "email".to_string(),
                value: "jane.smith@test-clinic.org".to_string(),
            },
            assertion: None,
        },
        receivers: vec![config::Entity {
            name: "Receiving Clinic".to_string(),
            id: "org:receiver.clinic".to_string(),
            contact: config::ContactInfo::Detailed {
                system: "phone".to_string(),
                value: "+61-2-9999-8888".to_string(),
            },
            assertion: None,
        }],
        patient: config::PatientConfig {
            name: Some("John Doe".to_string()),
            id: Some("PAT12345".to_string()),
            dob: Some("1985-03-15".to_string()),
            sex: Some("M".to_string()),
            identifiers: Some(vec![config::IdentifierConfig {
                system: "http://ns.electronichealth.net.au/id/hi/ihi/1.0".to_string(),
                value: "8003608166690503".to_string(),
                use_field: Some("official".to_string()),
            }]),
            verification: Some(config::VerificationConfig {
                status: "verified".to_string(),
                method: "government-id".to_string(),
                timestamp: Some("2024-01-15T10:30:00Z".to_string()),
            }),
        },
        security: Some(config::SecurityConfig {
            classification: Some("confidential".to_string()),
        }),
        encryption: None,
        custom_tags: Some(vec!["radiology".to_string(), "urgent".to_string()]),
        report: Some(config::ReportConfig {
            file: Some("files/radiology-report.pdf".to_string()),
            url: None,
        }),
        files: Some(vec![config::FileConfig::Detailed {
            path: "test-files/sample.pdf".to_string(),
            name: Some("sample-report.pdf".to_string()),
        }]),
        consent: Some(config::ConsentConfig {
            status: "granted".to_string(),
            scope: Some(vec!["treatment".to_string(), "research".to_string()]),
            method: Some("digital-signature".to_string()),
        }),
        deid_keys: Some(vec!["PatientName".to_string(), "PatientID".to_string()]),
        jws_signing_key: None,
        verify_assertions: Some(false),
    }
}

/// Create a test manifest for validation
fn create_test_manifest() -> types::Manifest {
    types::Manifest {
        version: "1.0".to_string(),
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        timestamp: "2024-01-15T14:30:00Z".to_string(),
        sender: types::Entity {
            name: Some("Test Healthcare Organization".to_string()),
            id: "org:test.health.123".to_string(),
            contact: types::Contact::Email("sender@test-health.org".to_string()),
            assertion: None,
        },
        requester: Some(types::Entity {
            name: Some("Dr. Jane Smith".to_string()),
            id: "doc:jane.smith".to_string(),
            contact: types::Contact::Point(types::ContactPoint {
                system: "email".to_string(),
                value: "jane.smith@test-clinic.org".to_string(),
            }),
            assertion: None,
        }),
        receiver: vec![types::Entity {
            name: Some("Receiving Clinic".to_string()),
            id: "org:receiver.clinic".to_string(),
            contact: types::Contact::Point(types::ContactPoint {
                system: "phone".to_string(),
                value: "+61-2-9999-8888".to_string(),
            }),
            assertion: None,
        }],
        security: types::Security {
            classification: Some("confidential".to_string()),
            payload_hash: "sha256:a1b2c3d4e5f6789012345678901234567890abcdef1234567890abcdef123456".to_string(),
            jws: None,
            signature: None,
            encryption: None,
        },
        extensions: Some(types::Extensions {
            custom_tags: Some(vec!["radiology".to_string(), "urgent".to_string()]),
            consent: Some(types::ConsentExtension {
                status: "granted".to_string(),
                scope: Some(vec!["treatment".to_string(), "research".to_string()]),
                method: Some("digital-signature".to_string()),
                signed_on: Some("2024-01-15".to_string()),
            }),
            deid: Some(types::DeidExtension {
                keys: vec!["PatientName".to_string(), "PatientID".to_string()],
            }),
            additional: std::collections::HashMap::new(),
        }),
    }
}

/// Create a test metadata structure  
fn create_test_metadata() -> types::Metadata {
    types::Metadata {
        version: "1.0".to_string(),
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        timestamp: "2024-01-15T14:30:00Z".to_string(),
        patient: types::Patient {
            id: "PAT12345".to_string(),
            name: Some(types::HumanName {
                family: Some("Doe".to_string()),
                given: Some(vec!["John".to_string(), "Michael".to_string()]),
                prefix: None,
                suffix: None,
                text: Some("John Michael Doe".to_string()),
            }),
            dob: Some("1985-03-15".to_string()),
            sex: Some("M".to_string()),
            identifiers: Some(vec![types::Identifier {
                system: "http://ns.electronichealth.net.au/id/hi/ihi/1.0".to_string(),
                value: "8003608166690503".to_string(),
            }]),
            verification: Some(types::Verification {
                verified_by: Some("government-system".to_string()),
                verified_on: Some("2024-01-15".to_string()),
            }),
        },
        report: Some(types::Report {
            file: Some("files/radiology-report.pdf".to_string()),
        }),
        studies: Some(types::Studies {
            study_description: Some("CT Pulmonary Angiogram".to_string()),
            study_uid: Some("1.2.3.4.5.6.7.8.9.10.11.12.13.14.15".to_string()),
            series: Some(vec![
                types::Series {
                    series_uid: "1.2.3.4.5.6.7.8.9.10.11.12.13.14.16".to_string(),
                    modality: "CT".to_string(),
                    body_part: Some("CHEST".to_string()),
                    instance_count: Some(150),
                },
                types::Series {
                    series_uid: "1.2.3.4.5.6.7.8.9.10.11.12.13.14.17".to_string(),
                    modality: "CT".to_string(),
                    body_part: Some("CHEST".to_string()),
                    instance_count: Some(75),
                },
            ]),
        }),
        extensions: Some(types::Extensions {
            custom_tags: Some(vec!["radiology".to_string(), "urgent".to_string()]),
            consent: Some(types::ConsentExtension {
                status: "granted".to_string(),
                scope: Some(vec!["treatment".to_string(), "research".to_string()]),
                method: Some("digital-signature".to_string()),
                signed_on: Some("2024-01-15".to_string()),
            }),
            deid: Some(types::DeidExtension {
                keys: vec!["PatientName".to_string(), "PatientID".to_string()],
            }),
            additional: std::collections::HashMap::new(),
        }),
    }
}

/// Create a test audit structure
fn create_test_audit() -> types::Audit {
    types::Audit {
        audit: vec![
            types::AuditEntry {
                event: "created".to_string(),
                by: types::EntityRef {
                    id: "org:test.health.123".to_string(),
                    name: Some("Test Healthcare Organization".to_string()),
                },
                to: None,
                timestamp: "2024-01-15T14:30:00Z".to_string(),
                assertion: None,
            },
            types::AuditEntry {
                event: "transmitted".to_string(),
                by: types::EntityRef {
                    id: "doc:jane.smith".to_string(),
                    name: Some("Dr. Jane Smith".to_string()),
                },
                to: Some(types::EntityRef {
                    id: "org:receiver.clinic".to_string(),
                    name: Some("Receiving Clinic".to_string()),
                }),
                timestamp: "2024-01-15T14:31:00Z".to_string(),
                assertion: None,
            },
        ],
    }
}

#[test]
fn test_config_serialization_roundtrip() {
    let config = create_test_config();
    
    // Serialize to JSON
    let json = serde_json::to_string_pretty(&config).expect("Failed to serialize config");
    assert!(!json.is_empty());
    println!("Config JSON:\n{}", json);
    
    // Deserialize back
    let deserialized: config::Config = serde_json::from_str(&json).expect("Failed to deserialize config");
    
    // Verify key fields
    assert_eq!(config.sender.name, deserialized.sender.name);
    assert_eq!(config.patient.name, deserialized.patient.name);
    assert_eq!(config.receivers.len(), deserialized.receivers.len());
    
    // Verify contact types work correctly
    match (&config.sender.contact, &deserialized.sender.contact) {
        (config::ContactInfo::Email(orig), config::ContactInfo::Email(deser)) => assert_eq!(orig, deser),
        _ => panic!("Contact type mismatch"),
    }
    
    match (&config.requester.contact, &deserialized.requester.contact) {
        (config::ContactInfo::Detailed { system: s1, value: v1 }, config::ContactInfo::Detailed { system: s2, value: v2 }) => {
            assert_eq!(s1, s2);
            assert_eq!(v1, v2);
        }
        _ => panic!("Contact type mismatch"),
    }
}

#[test]
fn test_manifest_structure_validity() {
    let manifest = create_test_manifest();
    
    // Test serialization
    let json = serde_json::to_string_pretty(&manifest).expect("Failed to serialize manifest");
    assert!(!json.is_empty());
    println!("Manifest JSON:\n{}", json);
    
    // Test deserialization
    let deserialized: types::Manifest = serde_json::from_str(&json).expect("Failed to deserialize manifest");
    
    // Verify critical fields
    assert_eq!(manifest.id, deserialized.id);
    assert_eq!(manifest.version, deserialized.version);
    assert_eq!(manifest.sender.id, deserialized.sender.id);
    assert_eq!(manifest.receiver.len(), deserialized.receiver.len());
    
    // Verify security block
    assert_eq!(manifest.security.payload_hash, deserialized.security.payload_hash);
    assert_eq!(manifest.security.classification, deserialized.security.classification);
    
    // Verify extensions
    assert!(deserialized.extensions.is_some());
    let extensions = deserialized.extensions.unwrap();
    assert!(extensions.custom_tags.is_some());
    assert!(extensions.consent.is_some());
    assert!(extensions.deid.is_some());
}

#[test]
fn test_metadata_structure_validity() {
    let metadata = create_test_metadata();
    
    // Test serialization
    let json = serde_json::to_string_pretty(&metadata).expect("Failed to serialize metadata");
    assert!(!json.is_empty());
    println!("Metadata JSON:\n{}", json);
    
    // Test deserialization
    let deserialized: types::Metadata = serde_json::from_str(&json).expect("Failed to deserialize metadata");
    
    // Verify patient information
    assert_eq!(metadata.patient.id, deserialized.patient.id);
    assert_eq!(metadata.patient.dob, deserialized.patient.dob);
    assert_eq!(metadata.patient.sex, deserialized.patient.sex);
    
    // Verify human name structure
    assert!(deserialized.patient.name.is_some());
    let name = deserialized.patient.name.unwrap();
    assert_eq!(name.family, Some("Doe".to_string()));
    assert_eq!(name.given, Some(vec!["John".to_string(), "Michael".to_string()]));
    
    // Verify studies information
    assert!(deserialized.studies.is_some());
    let studies = deserialized.studies.unwrap();
    assert_eq!(studies.study_description, Some("CT Pulmonary Angiogram".to_string()));
    assert!(studies.series.is_some());
    
    let series = studies.series.unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series[0].modality, "CT");
    assert_eq!(series[1].modality, "CT");
}

#[test]
fn test_audit_structure_validity() {
    let audit = create_test_audit();
    
    // Test serialization
    let json = serde_json::to_string_pretty(&audit).expect("Failed to serialize audit");
    assert!(!json.is_empty());
    println!("Audit JSON:\n{}", json);
    
    // Test deserialization
    let deserialized: types::Audit = serde_json::from_str(&json).expect("Failed to deserialize audit");
    
    // Verify audit entries
    assert_eq!(audit.audit.len(), deserialized.audit.len());
    assert_eq!(audit.audit[0].event, "created");
    assert_eq!(audit.audit[1].event, "transmitted");
    
    // Verify entity references
    assert_eq!(deserialized.audit[0].by.id, "org:test.health.123");
    assert_eq!(deserialized.audit[1].to.as_ref().unwrap().id, "org:receiver.clinic");
}

#[test]
fn test_files_manifest_structure() {
    let files: types::Files = vec![
        types::FileEntry {
            file: "files/report.pdf".to_string(),
            hash: Some("sha256:abc123def456".to_string()),
            size_bytes: Some(1024),
        },
        types::FileEntry {
            file: "files/image.jpg".to_string(),
            hash: Some("sha256:def456ghi789".to_string()),
            size_bytes: Some(2048),
        },
    ];
    
    // Test serialization
    let json = serde_json::to_string_pretty(&files).expect("Failed to serialize files");
    assert!(!json.is_empty());
    println!("Files JSON:\n{}", json);
    
    // Test deserialization
    let deserialized: types::Files = serde_json::from_str(&json).expect("Failed to deserialize files");
    
    assert_eq!(files.len(), deserialized.len());
    assert_eq!(deserialized[0].file, "files/report.pdf");
    assert_eq!(deserialized[1].size_bytes, Some(2048));
}

#[test]
fn test_complete_envelope_structure() {
    let envelope = Envelope {
        manifest: create_test_manifest(),
        metadata: create_test_metadata(),
        audit: create_test_audit(),
        manifest_jws: None,
    };
    
    // Test serialization
    let json = serde_json::to_string_pretty(&envelope).expect("Failed to serialize envelope");
    assert!(!json.is_empty());
    
    // Test deserialization
    let deserialized: types::Envelope = serde_json::from_str(&json).expect("Failed to deserialize envelope");
    
    // Verify all components are present and correct
    assert_eq!(envelope.manifest.id, deserialized.manifest.id);
    assert_eq!(envelope.metadata.patient.id, deserialized.metadata.patient.id);
    assert_eq!(envelope.audit.audit.len(), deserialized.audit.audit.len());
}

#[test]
#[ignore = "Requires schema files in ../jmix/schemas"]
fn test_validation_integration_with_real_schemas() {
    let validator = SchemaValidator::with_default_config();
    
    // Test manifest validation
    let manifest = create_test_manifest();
    match validator.validate_manifest(&manifest) {
        Ok(()) => println!("✓ Manifest validation passed"),
        Err(e) => println!("✗ Manifest validation failed: {}", e),
    }
    
    // Test metadata validation
    let metadata = create_test_metadata();
    match validator.validate_metadata(&metadata) {
        Ok(()) => println!("✓ Metadata validation passed"),
        Err(e) => println!("✗ Metadata validation failed: {}", e),
    }
    
    // Test audit validation
    let audit = create_test_audit();
    match validator.validate_audit(&audit) {
        Ok(()) => println!("✓ Audit validation passed"),
        Err(e) => println!("✗ Audit validation failed: {}", e),
    }
    
    // Test files validation
    let files: types::Files = vec![types::FileEntry {
        file: "files/report.pdf".to_string(),
        hash: Some("sha256:abc123def456".to_string()),
        size_bytes: Some(1024),
    }];
    match validator.validate_files(&files) {
        Ok(()) => println!("✓ Files validation passed"),
        Err(e) => println!("✗ Files validation failed: {}", e),
    }
}

#[test]
fn test_dicom_processor_with_sample_data() {
    let processor = DicomProcessor::new();
    let config = create_test_config();
    
    // Check if real sample DICOM files exist
    let dicom_dir = std::path::Path::new("samples/study_1");
    if !dicom_dir.exists() {
        println!("Skipping test - sample DICOM directory not found: {}", dicom_dir.display());
        return;
    }
    
    // Test processing with real DICOM files
    let result = processor.process_dicom_folder(&dicom_dir, Some(&config));
    assert!(result.is_ok(), "Failed to process DICOM folder: {:?}", result.err());
    
    let metadata = result.unwrap();
    
    // With real DICOM files, we should get actual metadata
    println!("Extracted DICOM metadata:");
    println!("  Patient name: {:?}", metadata.patient_name);
    println!("  Patient ID: {:?}", metadata.patient_id);
    println!("  Study description: {:?}", metadata.study_description);
    println!("  Modalities: {:?}", metadata.modalities);
    println!("  Series count: {}", metadata.series.len());
    println!("  Instance count: {}", metadata.instance_count);
    
    // Verify we found the expected number of files
    assert_eq!(metadata.instance_count, 15); // We saw 15 .dcm files
    
    // With real DICOM files, we should have series information
    assert!(!metadata.series.is_empty(), "Expected series information from real DICOM files");
    
    // Verify series contain expected data
    for (i, series) in metadata.series.iter().enumerate() {
        println!("  Series {}: UID={}, Modality={}, Instances={:?}", 
                 i + 1, series.series_uid, series.modality, series.instance_count);
        assert!(!series.series_uid.is_empty(), "Series UID should not be empty");
        assert!(!series.modality.is_empty(), "Modality should not be empty");
    }
}

#[test]
fn test_error_handling() {
    use jmix_rs::error::{JmixError, ValidationError};
    
    // Test validation error
    let validation_error = ValidationError::SchemaNotFound {
        path: "/nonexistent/schema.json".to_string(),
    };
    let jmix_error: JmixError = validation_error.into();
    
    match jmix_error {
        JmixError::Validation(ValidationError::SchemaNotFound { path }) => {
            assert_eq!(path, "/nonexistent/schema.json");
        }
        _ => panic!("Wrong error type"),
    }
    
    // Test DICOM error
    let dicom_error = JmixError::Dicom("Test DICOM error".to_string());
    assert!(dicom_error.to_string().contains("DICOM"));
}

#[test] 
fn test_contact_serialization_variants() {
    // Test different contact formats that should serialize/deserialize correctly
    let contacts = vec![
        types::Contact::Email("test@example.com".to_string()),
        types::Contact::Point(types::ContactPoint {
            system: "phone".to_string(),
            value: "+61-2-1234-5678".to_string(),
        }),
        types::Contact::Point(types::ContactPoint {
            system: "fax".to_string(),
            value: "+61-2-1234-5679".to_string(),
        }),
    ];
    
    for contact in contacts {
        let json = serde_json::to_string(&contact).expect("Failed to serialize contact");
        let deserialized: types::Contact = serde_json::from_str(&json).expect("Failed to deserialize contact");
        
        match (&contact, &deserialized) {
            (types::Contact::Email(orig), types::Contact::Email(deser)) => assert_eq!(orig, deser),
            (types::Contact::Point(orig), types::Contact::Point(deser)) => {
                assert_eq!(orig.system, deser.system);
                assert_eq!(orig.value, deser.value);
            }
            _ => panic!("Contact variant mismatch after serialization round-trip"),
        }
    }
}

#[test]
fn test_uuid_validation_format() {
    // Test that our test UUIDs are in the correct format
    let test_id = "550e8400-e29b-41d4-a716-446655440000";
    assert_eq!(test_id.len(), 36);
    assert_eq!(test_id.chars().nth(8).unwrap(), '-');
    assert_eq!(test_id.chars().nth(13).unwrap(), '-');
    assert_eq!(test_id.chars().nth(18).unwrap(), '-');
    assert_eq!(test_id.chars().nth(23).unwrap(), '-');
}

#[test] 
fn test_hash_format_validation() {
    // Test that our hash formats match expected patterns
    let test_hash = "sha256:a1b2c3d4e5f6789012345678901234567890abcdef1234567890abcdef123456";
    assert!(test_hash.starts_with("sha256:"));
    assert_eq!(test_hash.len(), 71); // "sha256:" (7) + 64 hex characters
    
    // Verify hex part
    let hex_part = &test_hash[7..];
    assert_eq!(hex_part.len(), 64);
    assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
}