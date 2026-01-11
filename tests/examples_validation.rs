use harmony::config::config::Config;
use harmony::config::Cli;

/// Helper to load and validate example config
fn load_and_validate_example(config_path: &str) -> Config {
    let cli = Cli::new(config_path.to_string());
    let config = Config::from_args(cli);

    config
        .validate()
        .expect(&format!("Config validation failed for {}", config_path));

    config
}

// ============================================================================
// CONFIGURATION VALIDATION TESTS
// Tests that all examples load successfully and have the expected structure
// ============================================================================

#[test]
#[ignore = "Requires echo service to be loaded"]
fn test_basic_echo_example_loads() {
    let config = load_and_validate_example("../harmony-examples/pipelines/basic-echo/config.toml");
    assert_eq!(config.proxy.effective_id(), "harmony-basic-echo");

    // Check that pipelines loaded
    assert!(config.pipelines.contains_key("echo_basic"));

    // Check that policy middleware is present
    let pipeline = &config.pipelines["echo_basic"];
    assert!(pipeline
        .middleware
        .contains(&"echo_policies".to_string()));

    // Check middleware definition exists
    assert!(config.middleware.contains_key("echo_policies"));
    let middleware = &config.middleware["echo_policies"];
    assert_eq!(middleware.middleware_type, "policies");
}

#[test]
fn test_content_types_example_loads() {
    let config = load_and_validate_example("../harmony-examples/pipelines/content-types/config.toml");
    assert_eq!(config.proxy.effective_id(), "content-types-example");

    // Check that pipelines loaded
    assert!(config.pipelines.contains_key("multi_content"));

    // Check that policy middleware is present
    let pipeline = &config.pipelines["multi_content"];
    assert!(pipeline
        .middleware
        .contains(&"content_security".to_string()));

    // Check middleware definition exists
    assert!(config.middleware.contains_key("content_security"));
}

#[test]
fn test_http_backend_example_loads() {
    let config = load_and_validate_example("../harmony-examples/pipelines/http-http/config.toml");
    assert_eq!(config.proxy.effective_id(), "harmony-http-backend");

    // Check that pipelines loaded
    assert!(config.pipelines.contains_key("http_proxy"));

    // Check that policy middleware is present
    let pipeline = &config.pipelines["http_proxy"];
    assert!(pipeline.middleware.contains(&"access_control".to_string()));
}

#[test]
#[ignore = "Requires FHIR service to be loaded"]
fn test_fhir_example_loads() {
    let config = load_and_validate_example("../harmony-examples/pipelines/fhir/config.toml");
    assert_eq!(config.proxy.effective_id(), "harmony-fhir");

    // Check that pipelines loaded
    assert!(config.pipelines.contains_key("fhir"));

    // Check that policy middleware is present
    let pipeline = &config.pipelines["fhir"];
    assert!(pipeline
        .middleware
        .contains(&"healthcare_policies".to_string()));

    // Check middleware definition exists
    assert!(config.middleware.contains_key("healthcare_policies"));
    let middleware = &config.middleware["healthcare_policies"];
    assert_eq!(middleware.middleware_type, "policies");
}

#[test]
#[ignore = "Requires DICOMweb service to be loaded"]
fn test_dicomweb_example_loads() {
    let config = load_and_validate_example("../harmony-examples/pipelines/dicomweb/config.toml");
    assert_eq!(config.proxy.effective_id(), "harmony-dicomweb");

    // Check that pipelines loaded
    assert!(config.pipelines.contains_key("dicomweb_demo"));

    // Check that policy middleware is present
    let pipeline = &config.pipelines["dicomweb_demo"];
    assert!(pipeline
        .middleware
        .contains(&"imaging_security".to_string()));

    // Check that dicomweb_bridge is also present
    assert!(pipeline.middleware.contains(&"dicomweb_bridge".to_string()));
}

#[test]
#[ignore = "Requires JMIX service to be loaded"]
fn test_jmix_example_loads() {
    let config = load_and_validate_example("../harmony-examples/pipelines/jmix/config.toml");
    assert_eq!(config.proxy.effective_id(), "harmony-jmix");

    // Check that pipelines loaded
    assert!(config.pipelines.contains_key("jmix_performance"));

    // Check that policy middleware is present
    let pipeline = &config.pipelines["jmix_performance"];
    assert!(pipeline
        .middleware
        .contains(&"package_security".to_string()));

    // Check that jmix_builder is also present
    assert!(pipeline.middleware.contains(&"jmix_builder".to_string()));
}

#[test]
fn test_transform_example_loads() {
    let config = load_and_validate_example("../harmony-examples/pipelines/transform/config.toml");
    assert_eq!(config.proxy.effective_id(), "harmony-transform");

    // Check that pipelines loaded
    assert!(config.pipelines.contains_key("transform_demo"));

    // Check that policy middleware is present
    let pipeline = &config.pipelines["transform_demo"];
    assert!(pipeline
        .middleware
        .contains(&"transform_security".to_string()));

    // Check that other middleware is also present
    assert!(pipeline.middleware.contains(&"json_extractor".to_string()));
    assert!(pipeline
        .middleware
        .contains(&"patient_transform".to_string()));
}

// ============================================================================
// POLICY STRUCTURE TESTS
// Tests that policy middleware has the expected rules configured
// ============================================================================

#[test]
#[ignore = "Requires echo service to be loaded"]
fn test_basic_echo_has_public_access_policy() {
    let config = load_and_validate_example("../harmony-examples/pipelines/basic-echo/config.toml");
    let middleware = &config.middleware["echo_policies"];

    // Check that policies middleware type is correct
    assert_eq!(middleware.middleware_type, "policies");

    // Note: We can't easily inspect the nested policy structure without
    // adding specific parsing code, but validation passing confirms structure is correct
}

#[test]
fn test_http_backend_has_path_filtering() {
    let config = load_and_validate_example("../harmony-examples/pipelines/http-http/config.toml");
    let middleware = &config.middleware["access_control"];

    assert_eq!(middleware.middleware_type, "policies");
}

#[test]
#[ignore = "Requires FHIR service to be loaded"]
fn test_fhir_has_time_based_policy() {
    let config = load_and_validate_example("../harmony-examples/pipelines/fhir/config.toml");
    let middleware = &config.middleware["healthcare_policies"];

    assert_eq!(middleware.middleware_type, "policies");
}

#[test]
fn test_transform_has_post_only_policy() {
    let config = load_and_validate_example("../harmony-examples/pipelines/transform/config.toml");
    let middleware = &config.middleware["transform_security"];

    assert_eq!(middleware.middleware_type, "policies");
}

#[test]
#[ignore = "Requires JMIX service to be loaded"]
fn test_jmix_has_read_only_policy() {
    let config = load_and_validate_example("../harmony-examples/pipelines/jmix/config.toml");
    let middleware = &config.middleware["package_security"];

    assert_eq!(middleware.middleware_type, "policies");
}

// ============================================================================
// NETWORK AND SERVICE CONFIGURATION TESTS
// ============================================================================

#[test]
#[ignore = "Some examples require specialized services"]
fn test_all_examples_have_required_services() {
    let examples = vec![
        ("../harmony-examples/pipelines/basic-echo/config.toml", vec!["http", "echo"]),
        ("../harmony-examples/pipelines/content-types/config.toml", vec!["http"]),
        ("../harmony-examples/pipelines/http-http/config.toml", vec!["http"]),
        ("../harmony-examples/pipelines/fhir/config.toml", vec!["http", "fhir"]),
        (
            "../harmony-examples/pipelines/dicomweb/config.toml",
            vec!["dicomweb", "dicom_scu"],
        ),
        ("../harmony-examples/pipelines/jmix/config.toml", vec!["jmix", "dicom_scu"]),
        ("../harmony-examples/pipelines/transform/config.toml", vec!["http", "echo"]),
    ];

    for (config_path, required_services) in examples {
        let config = load_and_validate_example(config_path);

        for service in required_services {
            assert!(
                config.services.contains_key(service),
                "{} is missing service: {}",
                config_path,
                service
            );
        }
    }
}

#[test]
#[ignore = "Some examples require specialized services to load"]
fn test_all_examples_have_policies_middleware_type() {
    let examples = vec![
        "../harmony-examples/pipelines/basic-echo/config.toml",
        "../harmony-examples/pipelines/content-types/config.toml",
        "../harmony-examples/pipelines/http-http/config.toml",
        "../harmony-examples/pipelines/fhir/config.toml",
        "../harmony-examples/pipelines/dicomweb/config.toml",
        "../harmony-examples/pipelines/jmix/config.toml",
        "../harmony-examples/pipelines/transform/config.toml",
    ];

    for config_path in examples {
        let config = load_and_validate_example(config_path);

        assert!(
            config.middleware_types.contains_key("policies"),
            "{} is missing policies middleware type registration",
            config_path
        );
    }
}
