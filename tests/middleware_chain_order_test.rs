//! Integration tests for middleware chain execution order
//!
//! These tests verify that:
//! - List format (middleware = [...]) reverses the right chain
//! - Split format (middleware.right = [...]) preserves the right chain order

use harmony::models::pipelines::config::PipelineMiddleware;

#[test]
fn test_list_format_should_reverse_right() {
    // For List format, right chain should be reversed
    let middleware_config = PipelineMiddleware::List(vec![
        "first".to_string(),
        "second".to_string(),
        "third".to_string(),
    ]);

    // Verify should_reverse_right returns true
    assert_eq!(middleware_config.should_reverse_right(), true);

    // Verify chains are the same (will be reversed during execution)
    assert_eq!(
        middleware_config.left_chain(),
        vec!["first", "second", "third"]
    );
    assert_eq!(
        middleware_config.right_chain(),
        vec!["first", "second", "third"]
    );
}

#[test]
fn test_split_format_should_not_reverse_right() {
    // For Split format, right chain should NOT be reversed
    let middleware_config = PipelineMiddleware::Split {
        left: vec!["auth".to_string()],
        right: vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ],
    };

    // Verify should_reverse_right returns false
    assert_eq!(middleware_config.should_reverse_right(), false);

    // Verify chains are different
    assert_eq!(middleware_config.left_chain(), vec!["auth"]);
    assert_eq!(
        middleware_config.right_chain(),
        vec!["first", "second", "third"]
    );
}

#[test]
fn test_empty_list_should_indicate_reversal() {
    // Empty list should still indicate reversal (though it won't matter)
    let middleware_config = PipelineMiddleware::List(vec![]);
    assert_eq!(middleware_config.should_reverse_right(), true);
}

#[test]
fn test_split_with_empty_right_should_not_reverse() {
    // Split format with empty right chain should not reverse
    let middleware_config = PipelineMiddleware::Split {
        left: vec!["a".to_string()],
        right: vec![],
    };
    assert_eq!(middleware_config.should_reverse_right(), false);
}

#[test]
fn test_split_with_only_right_should_not_reverse() {
    // Split format with only right chain should not reverse
    let middleware_config = PipelineMiddleware::Split {
        left: vec![],
        right: vec!["b".to_string()],
    };
    assert_eq!(middleware_config.should_reverse_right(), false);
}
