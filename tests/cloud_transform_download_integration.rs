use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// Test helper to extract transform IDs from TOML configuration
// This duplicates the logic from cloud_poller.rs for testing purposes
fn extract_transform_ids_test(toml_config: &str) -> Result<Vec<String>, String> {
    use std::collections::HashSet;
    
    let config: toml::Value = toml::from_str(toml_config)
        .map_err(|e| format!("Failed to parse TOML config: {}", e))?;

    let mut transform_ids = HashSet::new();

    if let Some(middleware) = config.get("middleware").and_then(|v| v.as_table()) {
        for (_middleware_name, middleware_config) in middleware {
            if let Some(middleware_type) = middleware_config.get("type").and_then(|v| v.as_str()) {
                if middleware_type == "transform" {
                    if let Some(spec_path) = middleware_config
                        .get("options")
                        .and_then(|opts| opts.get("spec_path"))
                        .and_then(|v| v.as_str())
                    {
                        let filename = spec_path
                            .rsplit('/')
                            .next()
                            .unwrap_or(spec_path);
                        
                        let transform_id = filename.strip_suffix(".json").unwrap_or(filename);
                        
                        if !transform_id.is_empty() {
                            transform_ids.insert(transform_id.to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(transform_ids.into_iter().collect())
}

#[test]
fn test_extract_transform_ids_no_transforms() {
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080
"#;

    let result = extract_transform_ids_test(config);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 0);
}

#[test]
fn test_extract_transform_ids_single_transform() {
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[middleware.patient_transform]
type = "transform"

[middleware.patient_transform.options]
spec_path = "01k81xczrw551e1qj9rgrf0319.json"
apply = "both"
fail_on_error = true

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080
"#;

    let result = extract_transform_ids_test(config);
    assert!(result.is_ok());
    
    let ids = result.unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&"01k81xczrw551e1qj9rgrf0319".to_string()));
}

#[test]
fn test_extract_transform_ids_multiple_transforms() {
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[middleware.patient_transform]
type = "transform"

[middleware.patient_transform.options]
spec_path = "01k81xczrw551e1qj9rgrf0319.json"

[middleware.study_transform]
type = "transform"

[middleware.study_transform.options]
spec_path = "01k81xgtn1hnbkfseyd82nar0m.json"

[middleware.other_middleware]
type = "basic_auth"
username = "test"
password = "test"

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080
"#;

    let result = extract_transform_ids_test(config);
    assert!(result.is_ok());
    
    let ids = result.unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"01k81xczrw551e1qj9rgrf0319".to_string()));
    assert!(ids.contains(&"01k81xgtn1hnbkfseyd82nar0m".to_string()));
}

#[test]
fn test_extract_transform_ids_with_path() {
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[middleware.patient_transform]
type = "transform"

[middleware.patient_transform.options]
spec_path = "subfolder/01k81xczrw551e1qj9rgrf0319.json"
"#;

    let result = extract_transform_ids_test(config);
    assert!(result.is_ok());
    
    let ids = result.unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&"01k81xczrw551e1qj9rgrf0319".to_string()));
}

#[test]
fn test_extract_transform_ids_without_extension() {
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[middleware.patient_transform]
type = "transform"

[middleware.patient_transform.options]
spec_path = "01k81xczrw551e1qj9rgrf0319"
"#;

    let result = extract_transform_ids_test(config);
    assert!(result.is_ok());
    
    let ids = result.unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&"01k81xczrw551e1qj9rgrf0319".to_string()));
}

#[test]
fn test_extract_transform_ids_duplicate_transforms() {
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[middleware.transform1]
type = "transform"

[middleware.transform1.options]
spec_path = "01k81xczrw551e1qj9rgrf0319.json"

[middleware.transform2]
type = "transform"

[middleware.transform2.options]
spec_path = "01k81xczrw551e1qj9rgrf0319.json"
"#;

    let result = extract_transform_ids_test(config);
    assert!(result.is_ok());
    
    // Should deduplicate - same transform used twice
    let ids = result.unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&"01k81xczrw551e1qj9rgrf0319".to_string()));
}

#[test]
fn test_extract_transform_ids_invalid_toml() {
    let config = r#"
[proxy
id = "missing-bracket"
"#;

    let result = extract_transform_ids_test(config);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to parse TOML"));
}

#[test]
fn test_extract_transform_ids_mixed_middleware() {
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[middleware.auth]
type = "jwt_auth"
public_key_path = "/keys/jwt.pem"

[middleware.transform_left]
type = "transform"

[middleware.transform_left.options]
spec_path = "patient_to_fhir.json"
apply = "left"

[middleware.passthrough]
type = "passthrough"

[middleware.transform_right]
type = "transform"

[middleware.transform_right.options]
spec_path = "response_filter.json"
apply = "right"
"#;

    let result = extract_transform_ids_test(config);
    assert!(result.is_ok());
    
    let ids = result.unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"patient_to_fhir".to_string()));
    assert!(ids.contains(&"response_filter".to_string()));
}

#[tokio::test]
async fn test_transform_file_write_and_read() {
    let temp_dir = TempDir::new().unwrap();
    let transforms_dir = temp_dir.path().join("transforms");
    
    // Create transforms directory
    fs::create_dir_all(&transforms_dir).expect("Failed to create transforms dir");
    
    // Write a mock JOLT spec
    let transform_id = "01k81xczrw551e1qj9rgrf0319";
    let jolt_spec = r#"[
  {
    "operation": "shift",
    "spec": {
      "PatientID": "resource.identifier[0].value",
      "PatientName": "resource.name[0].family"
    }
  }
]"#;
    
    let transform_path = transforms_dir.join(format!("{}.json", transform_id));
    fs::write(&transform_path, jolt_spec).expect("Failed to write transform");
    
    // Verify file exists and content matches
    assert!(transform_path.exists());
    let content = fs::read_to_string(&transform_path).expect("Failed to read transform");
    assert_eq!(content, jolt_spec);
    
    // Verify it's valid JSON
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(parsed.is_ok(), "Transform spec should be valid JSON");
}

#[tokio::test]
async fn test_transform_file_overwrite() {
    let temp_dir = TempDir::new().unwrap();
    let transforms_dir = temp_dir.path().join("transforms");
    
    fs::create_dir_all(&transforms_dir).expect("Failed to create transforms dir");
    
    let transform_id = "01k81xczrw551e1qj9rgrf0319";
    let transform_path = transforms_dir.join(format!("{}.json", transform_id));
    
    // Write initial version
    let old_spec = r#"[{"operation": "shift"}]"#;
    fs::write(&transform_path, old_spec).expect("Failed to write initial transform");
    
    // Verify initial content
    let content = fs::read_to_string(&transform_path).expect("Failed to read transform");
    assert_eq!(content, old_spec);
    
    // Overwrite with new version
    let new_spec = r#"[{"operation": "default"}]"#;
    fs::write(&transform_path, new_spec).expect("Failed to overwrite transform");
    
    // Verify new content replaced old
    let content = fs::read_to_string(&transform_path).expect("Failed to read transform");
    assert_eq!(content, new_spec);
    assert_ne!(content, old_spec);
}

#[tokio::test]
async fn test_transforms_directory_creation() {
    let temp_dir = TempDir::new().unwrap();
    let transforms_dir = temp_dir.path().join("transforms");
    
    // Verify directory doesn't exist initially
    assert!(!transforms_dir.exists());
    
    // Create directory
    let result = fs::create_dir_all(&transforms_dir);
    assert!(result.is_ok());
    
    // Verify directory was created
    assert!(transforms_dir.exists());
    assert!(transforms_dir.is_dir());
}

#[tokio::test]
async fn test_nested_transforms_directory() {
    let temp_dir = TempDir::new().unwrap();
    let nested_dir = temp_dir.path().join("config").join("transforms");
    
    // Create nested directory structure
    let result = fs::create_dir_all(&nested_dir);
    assert!(result.is_ok());
    
    // Verify both levels exist
    assert!(temp_dir.path().join("config").exists());
    assert!(nested_dir.exists());
    
    // Write transform to nested directory
    let transform_path = nested_dir.join("test_transform.json");
    fs::write(&transform_path, "{}").expect("Failed to write transform");
    assert!(transform_path.exists());
}

#[tokio::test]
async fn test_transform_filename_generation() {
    // Test various transform ID formats
    let test_cases = vec![
        ("01k81xczrw551e1qj9rgrf0319", "01k81xczrw551e1qj9rgrf0319.json"),
        ("patient_to_fhir", "patient_to_fhir.json"),
        ("simple-transform", "simple-transform.json"),
        ("transform_123", "transform_123.json"),
    ];
    
    for (transform_id, expected_filename) in test_cases {
        let filename = format!("{}.json", transform_id);
        assert_eq!(filename, expected_filename);
        
        // Verify it creates a valid path
        let path = PathBuf::from(&filename);
        assert_eq!(path.extension().unwrap(), "json");
    }
}

#[tokio::test]
async fn test_multiple_transform_writes() {
    let temp_dir = TempDir::new().unwrap();
    let transforms_dir = temp_dir.path().join("transforms");
    
    fs::create_dir_all(&transforms_dir).expect("Failed to create transforms dir");
    
    // Write multiple transforms
    let transforms = vec![
        ("01k81xczrw551e1qj9rgrf0319", r#"[{"operation": "shift"}]"#),
        ("01k81xgtn1hnbkfseyd82nar0m", r#"[{"operation": "default"}]"#),
        ("patient_to_fhir", r#"[{"operation": "remove"}]"#),
    ];
    
    for (id, spec) in &transforms {
        let path = transforms_dir.join(format!("{}.json", id));
        fs::write(&path, spec).expect("Failed to write transform");
    }
    
    // Verify all files exist
    for (id, _) in &transforms {
        let path = transforms_dir.join(format!("{}.json", id));
        assert!(path.exists(), "Transform {} should exist", id);
    }
    
    // Verify directory contains exactly 3 files
    let entries: Vec<_> = fs::read_dir(&transforms_dir)
        .expect("Failed to read dir")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 3);
}

#[test]
fn test_config_with_transforms_validation() {
    // Test that a config with transforms is valid TOML
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[middleware.patient_transform]
id = "01k81xgtn1hnbkfseyd82nar0m"
type = "transform"

[middleware.patient_transform.options]
spec_path = "01k81xczrw551e1qj9rgrf0319.json"
apply = "both"
fail_on_error = true
inject_context = false

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080

[logging]
log_level = "info"
log_to_file = false

[storage]
backend = "filesystem"

[storage.options]
path = "./tmp"

[runbeam]
enabled = false

[management]
enabled = true
base_path = "/admin"
network = "default"
"#;

    // Verify it parses as valid TOML
    let parsed: Result<toml::Value, _> = toml::from_str(config);
    assert!(parsed.is_ok(), "Config with transforms should be valid TOML");
    
    // Verify transform extraction works
    let ids = extract_transform_ids_test(config).unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&"01k81xczrw551e1qj9rgrf0319".to_string()));
}

#[test]
fn test_config_change_with_transforms_structure() {
    use serde_json::json;
    
    // Test a ConfigChangeDetail with transform middleware
    let config_toml = r#"[proxy]
id = "gateway-123"
transforms_path = "transforms"

[middleware.patient_transform]
type = "transform"

[middleware.patient_transform.options]
spec_path = "01k81xczrw551e1qj9rgrf0319.json"

[network.default]
interface = "lo0"
enable_wireguard = false

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080
"#;
    
    let json = json!({
        "id": "01k8change123",
        "status": "queued",
        "type": "gateway",
        "gateway_id": "01k8gateway",
        "pipeline_id": null,
        "toml_config": config_toml,
        "metadata": {
            "gateway_name": "my-gateway",
            "generated_at": "2025-11-04T21:00:00+00:00"
        },
        "created_at": "2025-11-04T21:00:00.000000Z",
        "acknowledged_at": null,
        "applied_at": null,
        "failed_at": null,
        "error_message": null,
        "error_details": null
    });
    
    use runbeam_sdk::runbeam_api::ConfigChangeDetail;
    let detail: ConfigChangeDetail = serde_json::from_value(json).unwrap();
    
    // Verify structure
    assert_eq!(detail.id, "01k8change123");
    assert!(detail.toml_config.contains("patient_transform"));
    
    // Verify transform extraction from TOML
    let ids = extract_transform_ids_test(&detail.toml_config).unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&"01k81xczrw551e1qj9rgrf0319".to_string()));
}

#[tokio::test]
async fn test_transform_path_resolution() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("config");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    
    // Test relative path resolution
    let transforms_path = "transforms";
    let resolved = config_dir.join(transforms_path);
    
    assert_eq!(
        resolved,
        config_dir.join("transforms"),
        "Relative path should resolve to config dir"
    );
    
    // Create the resolved directory
    fs::create_dir_all(&resolved).expect("Failed to create transforms dir");
    assert!(resolved.exists());
    assert!(resolved.is_dir());
}

#[test]
fn test_transform_id_extraction_edge_cases() {
    // Test edge cases for transform ID extraction
    let test_cases = vec![
        ("transform.json", "transform"),
        ("01k81xczrw551e1qj9rgrf0319.json", "01k81xczrw551e1qj9rgrf0319"),
        ("path/to/transform.json", "transform"),
        ("multiple/nested/path/transform.json", "transform"),
        ("transform", "transform"), // No extension
        ("transform.spec.json", "transform.spec"), // Multiple dots
    ];
    
    for (input, expected_id) in test_cases {
        let filename = input.rsplit('/').next().unwrap_or(input);
        let id = filename.strip_suffix(".json").unwrap_or(filename);
        assert_eq!(id, expected_id, "Failed for input: {}", input);
    }
}

#[tokio::test]
async fn test_error_handling_write_failure() {
    let temp_dir = TempDir::new().unwrap();
    
    // Try to write to a location that doesn't exist and we don't create
    let invalid_path = temp_dir.path().join("nonexistent").join("transforms").join("test.json");
    
    // This should fail because parent directory doesn't exist
    let result = fs::write(&invalid_path, "{}");
    assert!(result.is_err(), "Write should fail for non-existent parent directory");
}

#[tokio::test]
async fn test_transform_spec_json_validity() {
    // Test various JOLT spec formats
    let valid_specs = vec![
        r#"[]"#,
        r#"[{"operation": "shift"}]"#,
        r#"[
  {
    "operation": "shift",
    "spec": {
      "PatientID": "resource.identifier[0].value"
    }
  },
  {
    "operation": "default",
    "spec": {
      "resourceType": "Patient"
    }
  }
]"#,
    ];
    
    for spec in valid_specs {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(spec);
        assert!(parsed.is_ok(), "JOLT spec should be valid JSON: {}", spec);
    }
}

#[test]
fn test_config_with_multiple_middleware_types() {
    let config = r#"
[proxy]
id = "test-gateway"
transforms_path = "transforms"

[middleware.auth]
type = "jwt_auth"
public_key_path = "/keys/jwt.pem"

[middleware.transform1]
type = "transform"

[middleware.transform1.options]
spec_path = "transform1.json"

[middleware.filter]
type = "path_filter"

[middleware.filter.options]
rules = ["/api/*"]

[middleware.transform2]
type = "transform"

[middleware.transform2.options]
spec_path = "transform2.json"
"#;

    let ids = extract_transform_ids_test(config).unwrap();
    
    // Should only extract transform middleware, not others
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"transform1".to_string()));
    assert!(ids.contains(&"transform2".to_string()));
}

// Note: Full end-to-end tests with mock RunbeamClient would require
// additional mocking infrastructure. These tests validate the core
// logic of transform ID extraction and file operations that the
// cloud poller uses.
