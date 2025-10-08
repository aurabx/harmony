use jmix_rs::{config, types, validation::SchemaValidator};
use std::fs;

/// Load sample configuration from JSON file
#[test]
fn test_load_sample_config() {
    let config_path = "samples/sample_config.json";
    if !std::path::Path::new(config_path).exists() {
        println!("Skipping test - sample config file not found: {}", config_path);
        return;
    }

    let content = fs::read_to_string(config_path)
        .expect("Failed to read sample config file");
    
    let config: config::Config = serde_json::from_str(&content)
        .expect("Failed to parse sample config");
    
    // Verify basic structure
    assert_eq!(config.version.as_deref(), Some("1.0"));
    assert_eq!(config.sender.id, "org:test.health.123");
    assert_eq!(config.patient.name.as_deref(), Some("John Doe"));
    
    // Verify contact variants are parsed correctly
    match config.sender.contact {
        config::ContactInfo::Email(email) => assert_eq!(email, "sender@test-health.org"),
        _ => panic!("Expected email contact for sender"),
    }
    
    match config.requester.contact {
        config::ContactInfo::Detailed { system, value } => {
            assert_eq!(system, "email");
            assert_eq!(value, "jane.smith@test-clinic.org");
        }
        _ => panic!("Expected detailed contact for requester"),
    }
    
    println!("✓ Sample config loaded and parsed successfully");
}

/// Load and validate sample manifest
#[test] 
fn test_load_sample_manifest() {
    let manifest_path = "samples/sample_manifest.json";
    if !std::path::Path::new(manifest_path).exists() {
        println!("Skipping test - sample manifest file not found: {}", manifest_path);
        return;
    }

    let content = fs::read_to_string(manifest_path)
        .expect("Failed to read sample manifest file");
    
    let manifest: types::Manifest = serde_json::from_str(&content)
        .expect("Failed to parse sample manifest");
    
    // Verify basic structure
    assert_eq!(manifest.version, "1.0");
    assert_eq!(manifest.id, "550e8400-e29b-41d4-a716-446655440000");
    assert_eq!(manifest.sender.id, "org:test.health.123");
    assert_eq!(manifest.receiver.len(), 1);
    
    // Verify security section
    assert_eq!(manifest.security.classification.as_deref(), Some("confidential"));
    assert!(manifest.security.payload_hash.starts_with("sha256:"));
    
    // Verify extensions
    assert!(manifest.extensions.is_some());
    let extensions = manifest.extensions.unwrap();
    assert!(extensions.custom_tags.is_some());
    assert!(extensions.consent.is_some());
    assert!(extensions.deid.is_some());
    
    println!("✓ Sample manifest loaded and parsed successfully");
}

/// Load and validate sample metadata
#[test]
fn test_load_sample_metadata() {
    let metadata_path = "samples/sample_metadata.json";
    if !std::path::Path::new(metadata_path).exists() {
        println!("Skipping test - sample metadata file not found: {}", metadata_path);
        return;
    }

    let content = fs::read_to_string(metadata_path)
        .expect("Failed to read sample metadata file");
    
    let metadata: types::Metadata = serde_json::from_str(&content)
        .expect("Failed to parse sample metadata");
    
    // Verify basic structure
    assert_eq!(metadata.version, "1.0");
    assert_eq!(metadata.patient.id, "PAT12345");
    
    // Verify patient name structure
    assert!(metadata.patient.name.is_some());
    let name = metadata.patient.name.unwrap();
    assert_eq!(name.family.as_deref(), Some("Doe"));
    assert_eq!(name.given.as_ref(), Some(&vec!["John".to_string(), "Michael".to_string()]));
    
    // Verify studies structure
    assert!(metadata.studies.is_some());
    let studies = metadata.studies.unwrap();
    assert_eq!(studies.study_description.as_deref(), Some("CT Pulmonary Angiogram"));
    assert!(studies.series.is_some());
    
    let series = studies.series.unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series[0].modality, "CT");
    assert_eq!(series[1].modality, "CT");
    
    println!("✓ Sample metadata loaded and parsed successfully");
}

/// Load and validate sample audit
#[test]
fn test_load_sample_audit() {
    let audit_path = "samples/sample_audit.json";
    if !std::path::Path::new(audit_path).exists() {
        println!("Skipping test - sample audit file not found: {}", audit_path);
        return;
    }

    let content = fs::read_to_string(audit_path)
        .expect("Failed to read sample audit file");
    
    let audit: types::Audit = serde_json::from_str(&content)
        .expect("Failed to parse sample audit");
    
    // Verify basic structure
    assert_eq!(audit.audit.len(), 2);
    assert_eq!(audit.audit[0].event, "created");
    assert_eq!(audit.audit[1].event, "transmitted");
    
    // Verify entity references
    assert_eq!(audit.audit[0].by.id, "org:test.health.123");
    assert_eq!(audit.audit[1].by.id, "doc:jane.smith");
    assert!(audit.audit[1].to.is_some());
    assert_eq!(audit.audit[1].to.as_ref().unwrap().id, "org:receiver.clinic");
    
    println!("✓ Sample audit loaded and parsed successfully");
}

/// Load and validate sample files manifest
#[test]
fn test_load_sample_files() {
    let files_path = "samples/sample_files.json";
    if !std::path::Path::new(files_path).exists() {
        println!("Skipping test - sample files file not found: {}", files_path);
        return;
    }

    let content = fs::read_to_string(files_path)
        .expect("Failed to read sample files file");
    
    let files: types::Files = serde_json::from_str(&content)
        .expect("Failed to parse sample files");
    
    // Verify basic structure
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].file, "files/radiology-report.pdf");
    assert!(files[0].hash.as_ref().unwrap().starts_with("sha256:"));
    assert_eq!(files[0].size_bytes, Some(1048576));
    
    // Verify all files have proper structure
    for file_entry in &files {
        assert!(!file_entry.file.is_empty());
        if let Some(hash) = &file_entry.hash {
            assert!(hash.starts_with("sha256:"));
            assert_eq!(hash.len(), 71); // "sha256:" + 64 hex chars
        }
        if let Some(size) = file_entry.size_bytes {
            assert!(size > 0);
        }
    }
    
    println!("✓ Sample files loaded and parsed successfully");
}

/// Test schema validation with sample files (requires schema files)
#[test]
#[ignore = "Requires schema files in ../jmix/schemas"]
fn test_validate_sample_files_with_schemas() {
    let validator = SchemaValidator::with_default_config();
    
    // Test manifest validation
    if let Ok(content) = fs::read_to_string("samples/sample_manifest.json") {
        let manifest: types::Manifest = serde_json::from_str(&content)
            .expect("Failed to parse sample manifest");
        
        match validator.validate_manifest(&manifest) {
            Ok(()) => println!("✓ Sample manifest validation passed"),
            Err(e) => {
                println!("✗ Sample manifest validation failed: {}", e);
                panic!("Manifest validation failed");
            }
        }
    }
    
    // Test metadata validation
    if let Ok(content) = fs::read_to_string("samples/sample_metadata.json") {
        let metadata: types::Metadata = serde_json::from_str(&content)
            .expect("Failed to parse sample metadata");
        
        match validator.validate_metadata(&metadata) {
            Ok(()) => println!("✓ Sample metadata validation passed"),
            Err(e) => {
                println!("✗ Sample metadata validation failed: {}", e);
                panic!("Metadata validation failed");
            }
        }
    }
    
    // Test audit validation
    if let Ok(content) = fs::read_to_string("samples/sample_audit.json") {
        let audit: types::Audit = serde_json::from_str(&content)
            .expect("Failed to parse sample audit");
        
        match validator.validate_audit(&audit) {
            Ok(()) => println!("✓ Sample audit validation passed"),
            Err(e) => {
                println!("✗ Sample audit validation failed: {}", e);
                panic!("Audit validation failed");
            }
        }
    }
    
    // Test files validation
    if let Ok(content) = fs::read_to_string("samples/sample_files.json") {
        let files: types::Files = serde_json::from_str(&content)
            .expect("Failed to parse sample files");
        
        match validator.validate_files(&files) {
            Ok(()) => println!("✓ Sample files validation passed"),
            Err(e) => {
                println!("✗ Sample files validation failed: {}", e);
                panic!("Files validation failed");
            }
        }
    }
}

/// Test configuration loading and round-trip conversion
#[test]
fn test_config_to_jmix_types_conversion() {
    let config_path = "samples/sample_config.json";
    if !std::path::Path::new(config_path).exists() {
        println!("Skipping test - sample config file not found: {}", config_path);
        return;
    }

    let content = fs::read_to_string(config_path)
        .expect("Failed to read sample config file");
    
    let config: config::Config = serde_json::from_str(&content)
        .expect("Failed to parse sample config");
    
    // Test conversion of config entities to JMIX entity types
    let manifest_sender = types::Entity {
        name: Some(config.sender.name.clone()),
        id: config.sender.id.clone(),
        contact: match &config.sender.contact {
            config::ContactInfo::Email(email) => types::Contact::Email(email.clone()),
            config::ContactInfo::Detailed { system, value } => types::Contact::Point(types::ContactPoint {
                system: system.clone(),
                value: value.clone(),
            }),
        },
        assertion: None,
    };
    
    // Verify the conversion worked
    assert_eq!(manifest_sender.name, Some("Test Healthcare Organization".to_string()));
    assert_eq!(manifest_sender.id, "org:test.health.123");
    
    match manifest_sender.contact {
        types::Contact::Email(email) => assert_eq!(email, "sender@test-health.org"),
        _ => panic!("Expected email contact"),
    }
    
    // Test patient data conversion
    let manifest_patient = types::Patient {
        id: config.patient.id.clone().unwrap_or_else(|| "unknown".to_string()),
        name: config.patient.name.as_ref().map(|name| {
            // Simple name parsing for test purposes
            let parts: Vec<&str> = name.split_whitespace().collect();
            types::HumanName {
                family: parts.last().map(|s| s.to_string()),
                given: if parts.len() > 1 { 
                    Some(parts[..parts.len()-1].iter().map(|s| s.to_string()).collect())
                } else { 
                    None 
                },
                prefix: None,
                suffix: None,
                text: Some(name.clone()),
            }
        }),
        dob: config.patient.dob.clone(),
        sex: config.patient.sex.clone(),
        identifiers: config.patient.identifiers.as_ref().map(|ids| {
            ids.iter().map(|id| types::Identifier {
                system: id.system.clone(),
                value: id.value.clone(),
            }).collect()
        }),
        verification: None,
    };
    
    assert_eq!(manifest_patient.id, "PAT12345");
    assert_eq!(manifest_patient.dob.as_deref(), Some("1985-03-15"));
    assert_eq!(manifest_patient.sex.as_deref(), Some("M"));
    
    println!("✓ Config to JMIX types conversion successful");
}

/// Test that sample data is consistent with expected formats
#[test] 
fn test_sample_data_consistency() {
    let sample_files = [
        "samples/sample_config.json",
        "samples/sample_manifest.json",
        "samples/sample_metadata.json", 
        "samples/sample_audit.json",
        "samples/sample_files.json",
    ];
    
    let mut found_files = 0;
    
    for file_path in &sample_files {
        if std::path::Path::new(file_path).exists() {
            found_files += 1;
            
            // Verify it's valid JSON
            let content = fs::read_to_string(file_path)
                .expect(&format!("Failed to read {}", file_path));
            
            let _json_value: serde_json::Value = serde_json::from_str(&content)
                .expect(&format!("Invalid JSON in {}", file_path));
            
            println!("✓ {} is valid JSON", file_path);
        } else {
            println!("⚠ {} not found (this is OK for some tests)", file_path);
        }
    }
    
    assert!(found_files > 0, "No sample files found - this suggests a problem with test setup");
    println!("✓ Found and validated {} sample files", found_files);
}