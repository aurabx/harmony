use harmony::models::pipelines::config::{Pipeline, PipelineMiddleware};

#[test]
fn test_pipeline_middleware_list_format() {
    let middleware = PipelineMiddleware::List(vec![
        "first".to_string(),
        "second".to_string(),
        "third".to_string(),
    ]);

    // Both chains should return the same list
    assert_eq!(middleware.left_chain(), vec!["first", "second", "third"]);
    assert_eq!(middleware.right_chain(), vec!["first", "second", "third"]);

    // is_empty should work correctly
    assert!(!middleware.is_empty());

    // contains should work
    assert!(middleware.contains(&"second".to_string()));
    assert!(!middleware.contains(&"fourth".to_string()));

    // len should work
    assert_eq!(middleware.len(), 3);

    // get should work
    assert_eq!(middleware.get(0), Some(&"first".to_string()));
    assert_eq!(middleware.get(1), Some(&"second".to_string()));
    assert_eq!(middleware.get(2), Some(&"third".to_string()));
    assert_eq!(middleware.get(3), None);
}

#[test]
fn test_pipeline_middleware_split_format() {
    let middleware = PipelineMiddleware::Split {
        left: vec!["auth".to_string(), "validate".to_string()],
        right: vec!["transform".to_string(), "log".to_string()],
    };

    // Chains should be different
    assert_eq!(middleware.left_chain(), vec!["auth", "validate"]);
    assert_eq!(middleware.right_chain(), vec!["transform", "log"]);

    // is_empty should return false
    assert!(!middleware.is_empty());

    // contains should check both chains
    assert!(middleware.contains(&"auth".to_string()));
    assert!(middleware.contains(&"log".to_string()));
    assert!(!middleware.contains(&"missing".to_string()));

    // len should be sum of both
    assert_eq!(middleware.len(), 4);

    // get should access combined vector (left first, then right)
    assert_eq!(middleware.get(0), Some(&"auth".to_string()));
    assert_eq!(middleware.get(1), Some(&"validate".to_string()));
    assert_eq!(middleware.get(2), Some(&"transform".to_string()));
    assert_eq!(middleware.get(3), Some(&"log".to_string()));
    assert_eq!(middleware.get(4), None);
}

#[test]
fn test_pipeline_middleware_split_left_only() {
    let middleware = PipelineMiddleware::Split {
        left: vec!["auth".to_string()],
        right: vec![],
    };

    assert_eq!(middleware.left_chain(), vec!["auth".to_string()]);
    let empty: Vec<String> = vec![];
    assert_eq!(middleware.right_chain(), empty);
    assert!(!middleware.is_empty());
    assert_eq!(middleware.len(), 1);
}

#[test]
fn test_pipeline_middleware_split_right_only() {
    let middleware = PipelineMiddleware::Split {
        left: vec![],
        right: vec!["log".to_string()],
    };

    let empty: Vec<String> = vec![];
    assert_eq!(middleware.left_chain(), empty);
    assert_eq!(middleware.right_chain(), vec!["log".to_string()]);
    assert!(!middleware.is_empty());
    assert_eq!(middleware.len(), 1);
}

#[test]
fn test_pipeline_middleware_empty_list() {
    let middleware = PipelineMiddleware::List(vec![]);

    let empty: Vec<String> = vec![];
    assert_eq!(middleware.left_chain(), empty.clone());
    assert_eq!(middleware.right_chain(), empty);
    assert!(middleware.is_empty());
    assert_eq!(middleware.len(), 0);
    assert_eq!(middleware.get(0), None);
}

#[test]
fn test_pipeline_middleware_empty_split() {
    let middleware = PipelineMiddleware::Split {
        left: vec![],
        right: vec![],
    };

    let empty: Vec<String> = vec![];
    assert_eq!(middleware.left_chain(), empty.clone());
    assert_eq!(middleware.right_chain(), empty);
    assert!(middleware.is_empty());
    assert_eq!(middleware.len(), 0);
    assert_eq!(middleware.get(0), None);
}

#[test]
fn test_pipeline_middleware_to_vec() {
    let list_mw = PipelineMiddleware::List(vec!["a".to_string(), "b".to_string()]);
    assert_eq!(list_mw.to_vec(), vec!["a", "b"]);

    let split_mw = PipelineMiddleware::Split {
        left: vec!["a".to_string(), "b".to_string()],
        right: vec!["c".to_string()],
    };
    assert_eq!(split_mw.to_vec(), vec!["a", "b", "c"]);
}

#[test]
fn test_pipeline_middleware_default() {
    let middleware = PipelineMiddleware::default();

    let empty: Vec<String> = vec![];
    assert_eq!(middleware.left_chain(), empty.clone());
    assert_eq!(middleware.right_chain(), empty);
    assert!(middleware.is_empty());
    assert_eq!(middleware.len(), 0);
}

#[test]
fn test_pipeline_with_list_middleware() {
    let pipeline = Pipeline {
        description: "Test pipeline".to_string(),
        networks: vec!["test".to_string()],
        endpoints: vec!["ep".to_string()],
        backends: vec!["be".to_string()],
        middleware: PipelineMiddleware::List(vec!["auth".to_string(), "transform".to_string()]),
    };

    assert_eq!(pipeline.middleware.len(), 2);
    assert_eq!(pipeline.middleware.left_chain(), vec!["auth".to_string(), "transform".to_string()]);
    assert_eq!(pipeline.middleware.right_chain(), vec!["auth".to_string(), "transform".to_string()]);
}

#[test]
fn test_pipeline_with_split_middleware() {
    let pipeline = Pipeline {
        description: "Test pipeline".to_string(),
        networks: vec!["test".to_string()],
        endpoints: vec!["ep".to_string()],
        backends: vec!["be".to_string()],
        middleware: PipelineMiddleware::Split {
            left: vec!["auth".to_string()],
            right: vec!["log".to_string()],
        },
    };

    assert_eq!(pipeline.middleware.len(), 2);
    assert_eq!(pipeline.middleware.left_chain(), vec!["auth".to_string()]);
    assert_eq!(pipeline.middleware.right_chain(), vec!["log".to_string()]);
}

#[test]
fn test_pipeline_middleware_deserialization_list_format() {
    // Simulate TOML: middleware = ["auth", "transform"]
    let json_str = r#"["auth", "transform"]"#;
    let middleware: PipelineMiddleware = serde_json::from_str(json_str).unwrap();

    assert_eq!(middleware.left_chain(), vec!["auth".to_string(), "transform".to_string()]);
    assert_eq!(middleware.right_chain(), vec!["auth".to_string(), "transform".to_string()]);
}

#[test]
fn test_pipeline_middleware_deserialization_split_format() {
    // Simulate TOML: [middleware]
    //                left = ["auth"]
    //                right = ["log"]
    let json_str = r#"{"left": ["auth"], "right": ["log"]}"#;
    let middleware: PipelineMiddleware = serde_json::from_str(json_str).unwrap();

    assert_eq!(middleware.left_chain(), vec!["auth".to_string()]);
    assert_eq!(middleware.right_chain(), vec!["log".to_string()]);
}

#[test]
fn test_pipeline_middleware_deserialization_split_left_only() {
    let json_str = r#"{"left": ["auth"]}"#;
    let middleware: PipelineMiddleware = serde_json::from_str(json_str).unwrap();

    assert_eq!(middleware.left_chain(), vec!["auth".to_string()]);
    let empty: Vec<String> = vec![];
    assert_eq!(middleware.right_chain(), empty);
}

#[test]
fn test_pipeline_middleware_deserialization_split_right_only() {
    let json_str = r#"{"right": ["log"]}"#;
    let middleware: PipelineMiddleware = serde_json::from_str(json_str).unwrap();

    let empty: Vec<String> = vec![];
    assert_eq!(middleware.left_chain(), empty);
    assert_eq!(middleware.right_chain(), vec!["log".to_string()]);
}

#[test]
fn test_pipeline_middleware_deserialization_empty_list() {
    let json_str = r#"[]"#;
    let middleware: PipelineMiddleware = serde_json::from_str(json_str).unwrap();

    assert!(middleware.is_empty());
}

#[test]
fn test_pipeline_middleware_deserialization_empty_split() {
    let json_str = r#"{"left": [], "right": []}"#;
    let middleware: PipelineMiddleware = serde_json::from_str(json_str).unwrap();

    assert!(middleware.is_empty());
}
