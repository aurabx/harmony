use harmony::config::config::Config;
use harmony::config::Cli;
use std::path::PathBuf;

/// Helper to load test configuration
fn load_test_config() -> Config {
    let config_path = format!(
        "{}/../harmony-examples/pipelines/fhir_dicom/config.toml",
        env!("CARGO_MANIFEST_DIR")
    );

    let cli = Cli::new(config_path);
    Config::from_args(cli)
}

/// Helper to get transform file path
fn get_transform_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../harmony-examples/pipelines/fhir_dicom/transforms")
        .join(name)
}

#[test]
fn test_configuration_loads_successfully() {
    // Test that the FHIR-DICOM configuration loads without errors
    let config = load_test_config();

    assert_eq!(config.proxy.effective_id(), "harmony-fhir-dicom");
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

    // Pipeline uses split middleware: left (request) + right (response)
    let expected_left = vec![
        "imagingstudy_filter",
        "query_to_target",
        "json_extractor",
        "fhir_dimse_meta",
        "debug_dump_meta",
        "fhir_to_dicom_transform",
    ];

    let expected_right = vec![
        "flatten_dicom",
        "dicom_to_fhir_bundle",
        "dump_final_bundle",
    ];

    // Total middleware count is left + right
    let expected_total = expected_left.len() + expected_right.len();
    assert_eq!(
        pipeline.middleware.len(),
        expected_total,
        "Should have {} middleware total",
        expected_total
    );

    // Verify left chain
    let left_chain = pipeline.middleware.left_chain();
    assert_eq!(left_chain, expected_left, "Left chain should match expected order");

    // Verify right chain
    let right_chain = pipeline.middleware.right_chain();
    assert_eq!(right_chain, expected_right, "Right chain should match expected order");

    // Verify combined ordering (left chain followed by right chain)
    let combined = pipeline.middleware.to_vec();
    let expected_combined = [expected_left, expected_right].concat();
    assert_eq!(combined, expected_combined, "Combined middleware should be left + right");
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
    // Verify transforms have context injection enabled (now default behavior)
    let config = load_test_config();

    // Only the transforms actually in the pipeline configuration
    let transform_middlewares = vec![
        "fhir_to_dicom_transform",
        "dicom_to_fhir_bundle",
    ];

    for name in transform_middlewares {
        let middleware = config
            .middleware
            .get(name)
            .unwrap_or_else(|| panic!("{} middleware should be defined", name));

        assert_eq!(
            middleware.middleware_type, "transform",
            "{} should be a transform middleware",
            name
        );
        // Note: inject_context is now always true and removed from config,
        // so we don't check for its presence in options map.
    }
}

#[test]
fn test_transform_files_exist_and_valid() {
    // Verify all transform files referenced in the pipeline exist and are valid JSON
    let transforms = vec![
        "query_to_target_details.json",
        "metadata_set_dimse_op.json",
        "fhir_to_dicom_params.json",
        "dicom_to_fhir_bundle.json",
    ];

    for transform_name in transforms {
        let path = get_transform_path(transform_name);

        assert!(path.exists(), "{} should exist", transform_name);

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
    let content =
        std::fs::read_to_string(&transform_path).expect("Should be able to read transform file");

    assert!(
        !content.contains("PID156695"),
        "Transform should not contain hardcoded PID156695"
    );

    // Parse and verify it uses context.target_details correctly
    let json: serde_json::Value =
        serde_json::from_str(&content).expect("Transform should be valid JSON");

    // Find the shift operation
    let shift_op = json
        .as_array()
        .and_then(|arr| arr.iter().find(|op| op["operation"] == "shift"))
        .expect("Transform should contain a shift operation");

    // Verify it accesses context.target_details.metadata for PatientID
    let patient_id_path = shift_op
        .pointer("/spec/context/target_details/metadata/PatientID")
        .expect("Transform should map context.target_details.metadata.PatientID");

    assert_eq!(
        patient_id_path.as_str(),
        Some("data.dimse_identifier.00100020.Value[0]"),
        "PatientID should be mapped from context to DICOM tag 00100020"
    );
}

// #[test]
// #[ignore] // enrich_with_jmix_urls middleware and transform are currently commented out in the pipeline
// fn test_jmix_url_pattern() {
//     // Verify JMIX URL enrichment transform has correct pattern
//     let transform_path = get_transform_path("enrich_with_jmix_urls.json");
//     let content =
//         std::fs::read_to_string(&transform_path).expect("Should be able to read transform file");
//
//     let json: serde_json::Value =
//         serde_json::from_str(&content).expect("Transform should be valid JSON");
//
//     // Find the modify operation
//     let modify_op = json
//         .as_array()
//         .and_then(|arr| {
//             arr.iter()
//                 .find(|op| op["operation"] == "modify-overwrite-beta")
//         })
//         .expect("Transform should contain a modify-overwrite-beta operation");
//
//     // Verify the concat expression builds the correct URL pattern
//     let jmix_expr = modify_op
//         .pointer("/spec/data/matches/*/_jmix_url")
//         .and_then(|v| v.as_str())
//         .expect("Transform should have _jmix_url expression");
//
//     assert!(
//         jmix_expr.contains("concat") && jmix_expr.contains("/api/jmix?studyInstanceUid="),
//         "Transform should use concat to build JMIX URL with correct base path"
//     );
//
//     assert!(
//         jmix_expr.contains("@(1,0020000D.Value.0)"),
//         "Transform should reference StudyInstanceUID from DICOM tag 0020000D"
//     );
// }

#[test]
fn test_dicom_to_fhir_includes_endpoints() {
    // Verify DICOM-to-FHIR transform includes endpoint structure
    let transform_path = get_transform_path("dicom_to_fhir_bundle.json");
    let content =
        std::fs::read_to_string(&transform_path).expect("Should be able to read transform file");

    let json: serde_json::Value =
        serde_json::from_str(&content).expect("Transform should be valid JSON");

    // Find the default operation that sets up the FHIR structure
    let default_op = json
        .as_array()
        .and_then(|arr| arr.iter().find(|op| op["operation"] == "default"))
        .expect("Transform should contain a default operation");

    // Verify Bundle structure
    assert_eq!(
        default_op
            .pointer("/spec/resourceType")
            .and_then(|v| v.as_str()),
        Some("Bundle"),
        "Transform should create FHIR Bundle"
    );

    // Verify type field exists
    assert_eq!(
        default_op
            .pointer("/spec/type")
            .and_then(|v| v.as_str()),
        Some("collection"),
        "Transform should set Bundle type to collection"
    );

    // Note: The current dicom_to_fhir_bundle.json does not include _jmix_url mapping.
    // That functionality would be handled by a separate enrich_jmix_urls middleware
    // which is currently commented out in the pipeline configuration.
    // This test verifies the basic FHIR Bundle structure is in place.
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

    // Should be dicom_scu for testing (based on example config)
    assert_eq!(
        backend.service, "dicom_scu",
        "Should use dicom_scu backend for testing"
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
    let content =
        std::fs::read_to_string(&transform_path).expect("Should be able to read transform file");

    let json: serde_json::Value =
        serde_json::from_str(&content).expect("Transform should be valid JSON");

    // Find the shift operation
    let shift_op = json
        .as_array()
        .and_then(|arr| arr.iter().find(|op| op["operation"] == "shift"))
        .expect("Transform should contain a shift operation");

    let expected_mappings = vec![
        ("patient", "PatientID", "/spec/query_params/patient/0"),
        (
            "identifier",
            "StudyInstanceUID",
            "/spec/query_params/identifier/0",
        ),
        ("modality", "Modality", "/spec/query_params/modality/0"),
        ("studyDate", "StudyDate", "/spec/query_params/studyDate/0"),
        (
            "accessionNumber",
            "AccessionNumber",
            "/spec/query_params/accessionNumber/0",
        ),
    ];

    for (query_param, metadata_field, pointer_path) in expected_mappings {
        let mapped_value = shift_op
            .pointer(pointer_path)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "Transform should map query_params.{} to metadata field",
                    query_param
                )
            });

        assert_eq!(
            mapped_value,
            format!("metadata.{}", metadata_field),
            "Query parameter '{}' should be mapped to metadata.{}",
            query_param,
            metadata_field
        );
    }
}
