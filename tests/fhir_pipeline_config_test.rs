use harmony::config::config::Config;
use harmony::config::Cli;
use std::path::PathBuf;

/// Helper to load test configuration
fn load_test_config() -> Config {
    let config_path = format!(
        "{}/examples/fhir_dicom/config.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    
    let cli = Cli::new(config_path);
    Config::from_args(cli)
}

/// Helper to get transform file path
fn get_transform_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/fhir_dicom/transforms")
        .join(name)
}

#[test]
fn test_configuration_loads_successfully() {
    // Test that the FHIR-DICOM configuration loads without errors
    let config = load_test_config();
    
    assert_eq!(config.proxy.id, "harmony-fhir-dicom");
    assert!(config.network.contains_key("default"));
    assert!(config.network.contains_key("management"));
}

#[test]
fn test_imagingstudy_pipeline_exists() {
    // Verify the imagingstudy_query pipeline is defined
    let config = load_test_config();
    
    assert!(
        config.pipelines.contains_key("imagingstudy_query"),
        "imagingstudy_query pipeline should exist"
    );
    
    let pipeline = config.pipelines.get("imagingstudy_query").unwrap();
    
    // Verify pipeline configuration
    assert_eq!(pipeline.networks, vec!["default".to_string()]);
    assert_eq!(pipeline.endpoints, vec!["fhir_imagingstudy_ep".to_string()]);
    assert_eq!(pipeline.backends, vec!["dicom_backend".to_string()]);
}

#[test]
fn test_middleware_chain_correct_order() {
    // Verify middleware are in the correct order
    let config = load_test_config();
    let pipeline = config.pipelines.get("imagingstudy_query").unwrap();
    
    let expected_middleware = vec![
        "imagingstudy_filter",
        "query_to_target",
        "json_extractor",
        "fhir_dimse_meta",
        "fhir_to_dicom_transform",
        "enrich_jmix_urls",
        "dicom_to_fhir_transform",
    ];
    
    assert_eq!(
        pipeline.middleware.len(),
        expected_middleware.len(),
        "Should have {} middleware",
        expected_middleware.len()
    );
    
    for (i, expected) in expected_middleware.iter().enumerate() {
        assert_eq!(
            pipeline.middleware.get(i).map(|s| s.as_str()),
            Some(*expected),
            "Middleware at position {} should be {}",
            i,
            expected
        );
    }
}

#[test]
fn test_query_to_target_middleware_configured() {
    // Verify query_to_target middleware has correct configuration
    let config = load_test_config();
    
    assert!(
        config.middleware.contains_key("query_to_target"),
        "query_to_target middleware should be defined"
    );
    
    let middleware = config.middleware.get("query_to_target").unwrap();
    assert_eq!(middleware.middleware_type, "metadata_transform");
    
    // Verify options
    let options = &middleware.options;
    assert!(
        options.contains_key("spec_path"),
        "Should have spec_path option"
    );
    assert_eq!(
        options.get("transform_target").and_then(|v| v.as_str()),
        Some("target_details"),
        "Should transform target_details"
    );
}

#[test]
fn test_context_injection_enabled() {
    // Verify transforms have context injection enabled
    let config = load_test_config();
    
    let transform_middlewares = vec![
        "fhir_to_dicom_transform",
        "enrich_jmix_urls",
        "dicom_to_fhir_transform",
    ];
    
    for name in transform_middlewares {
        let middleware = config.middleware.get(name)
            .unwrap_or_else(|| panic!("{} middleware should be defined", name));
        
        assert_eq!(
            middleware.middleware_type, "transform",
            "{} should be a transform middleware",
            name
        );
        
        let options = &middleware.options;
        assert_eq!(
            options.get("inject_context").and_then(|v| v.as_bool()),
            Some(true),
            "{} should have inject_context=true",
            name
        );
    }
}

#[test]
fn test_transform_files_exist_and_valid() {
    // Verify all transform files exist and are valid JSON
    let transforms = vec![
        "query_to_target_details.json",
        "metadata_set_dimse_op.json",
        "fhir_to_dicom_params.json",
        "enrich_with_jmix_urls.json",
        "dicom_to_imagingstudy_simple.json",
    ];
    
    for transform_name in transforms {
        let path = get_transform_path(transform_name);
        
        assert!(
            path.exists(),
            "{} should exist",
            transform_name
        );
        
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Should be able to read {}: {}", transform_name, e));
        
        let _json: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("{} should be valid JSON: {}", transform_name, e));
    }
}

#[test]
fn test_no_hardcoded_patient_id() {
    // Verify PID156695 is NOT hardcoded in the transform
    let transform_path = get_transform_path("fhir_to_dicom_params.json");
    let content = std::fs::read_to_string(&transform_path)
        .expect("Should be able to read transform file");
    
    assert!(
        !content.contains("PID156695"),
        "Transform should not contain hardcoded PID156695"
    );
    
    // Verify it uses context instead
    assert!(
        content.contains("context") && content.contains("target_details"),
        "Transform should use context.target_details"
    );
}

#[test]
fn test_jmix_url_pattern() {
    // Verify JMIX URL enrichment transform has correct pattern
    let transform_path = get_transform_path("enrich_with_jmix_urls.json");
    let content = std::fs::read_to_string(&transform_path)
        .expect("Should be able to read transform file");
    
    assert!(
        content.contains("/api/jmix?studyInstanceUid="),
        "Transform should generate JMIX API URLs with correct pattern"
    );
    
    assert!(
        content.contains("0020000D"),
        "Transform should extract StudyInstanceUID (tag 0020000D)"
    );
}

#[test]
fn test_dicom_to_fhir_includes_endpoints() {
    // Verify DICOM-to-FHIR transform includes endpoint structure
    let transform_path = get_transform_path("dicom_to_imagingstudy_simple.json");
    let content = std::fs::read_to_string(&transform_path)
        .expect("Should be able to read transform file");
    
    let content_lower = content.to_lowercase();
    
    assert!(
        content_lower.contains("endpoint"),
        "Transform should include endpoint structure"
    );
    
    assert!(
        content_lower.contains("imagingstudy"),
        "Transform should create ImagingStudy resources"
    );
    
    assert!(
        content_lower.contains("bundle"),
        "Transform should create FHIR Bundle"
    );
}

#[test]
fn test_backend_configuration() {
    // Verify DICOM backend is properly configured
    let config = load_test_config();
    
    assert!(
        config.backends.contains_key("dicom_backend"),
        "dicom_backend should be defined"
    );
    
    let backend = config.backends.get("dicom_backend").unwrap();
    
    // Should be mock_dicom for testing
    assert_eq!(
        backend.service, "mock_dicom",
        "Should use mock_dicom backend for testing"
    );
}

#[test]
fn test_fhir_endpoint_configuration() {
    // Verify FHIR endpoint is properly configured
    let config = load_test_config();
    
    assert!(
        config.endpoints.contains_key("fhir_imagingstudy_ep"),
        "fhir_imagingstudy_ep should be defined"
    );
    
    let endpoint = config.endpoints.get("fhir_imagingstudy_ep").unwrap();
    assert_eq!(endpoint.service, "fhir");
    
    // Verify path_prefix option
    if let Some(options) = &endpoint.options {
        assert_eq!(
            options.get("path_prefix").and_then(|v| v.as_str()),
            Some("/fhir"),
            "FHIR endpoint should have /fhir path prefix"
        );
    } else {
        panic!("Endpoint should have options");
    }
}

#[test]
fn test_query_params_mapped_to_dicom_tags() {
    // Verify query_to_target transform maps all expected parameters
    let transform_path = get_transform_path("query_to_target_details.json");
    let content = std::fs::read_to_string(&transform_path)
        .expect("Should be able to read transform file");
    
    let expected_params = vec![
        ("patient", "PatientID"),
        ("identifier", "StudyInstanceUID"),
        ("modality", "Modality"),
        ("studyDate", "StudyDate"),
        ("accessionNumber", "AccessionNumber"),
    ];
    
    for (query_param, metadata_field) in expected_params {
        assert!(
            content.contains(query_param),
            "Transform should handle {} query parameter",
            query_param
        );
        assert!(
            content.contains(metadata_field),
            "Transform should map to {} metadata field",
            metadata_field
        );
    }
}
