use harmony::models::pipelines::config::Pipeline;

#[test]
fn test_middleware_dotted_key_syntax() {
    let toml_content = r#"
[pipelines.test]
description = "Test"
networks = ["default"]
endpoints = ["ep1"]
backends = ["be1"]

middleware.left = ["a", "b"]
middleware.right = ["c", "d"]
"#;

    let parsed: toml::Value = toml::from_str(toml_content).expect("Failed to parse TOML");
    println!("Parsed TOML structure: {:#?}", parsed);

    // Check what structure we actually got
    let middleware_value = parsed
        .get("pipelines")
        .and_then(|p| p.get("test"))
        .and_then(|t| t.get("middleware"));

    println!("Middleware value: {:#?}", middleware_value);

    // Now try to deserialize into Pipeline
    #[derive(serde::Deserialize, Debug)]
    struct Wrapper {
        pipelines: std::collections::HashMap<String, Pipeline>,
    }

    let wrapper: Wrapper = toml::from_str(toml_content).expect("Failed to deserialize");
    let pipeline = wrapper.pipelines.get("test").expect("Pipeline not found");

    println!("Deserialized pipeline: {:#?}", pipeline);
    println!("Left chain: {:?}", pipeline.middleware.left_chain());
    println!("Right chain: {:?}", pipeline.middleware.right_chain());

    assert_eq!(pipeline.middleware.left_chain(), vec!["a", "b"]);
    assert_eq!(pipeline.middleware.right_chain(), vec!["c", "d"]);
}
