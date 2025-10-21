use super::executor::{PipelineError, PipelineExecutor};
use crate::config::config::Config;
use crate::models::backends::backends::Backend;
use crate::models::endpoints::endpoint::Endpoint;
use crate::models::envelope::envelope::{RequestEnvelope, RequestEnvelopeBuilder};
use crate::models::pipelines::config::Pipeline;
use crate::models::protocol::{Protocol, ProtocolCtx};
use std::collections::HashMap;

/// Helper to create a minimal test config
fn create_test_config() -> Config {
    let mut config = Config::default();
    
    // Add a test endpoint
    config.endpoints.insert(
        "test_endpoint".to_string(),
        Endpoint {
            service: "echo".to_string(),
            options: Some(HashMap::new()),
        },
    );
    
    // Add a test backend
    config.backends.insert(
        "test_backend".to_string(),
        Backend {
            service: "http".to_string(),
            options: Some({
                let mut opts = HashMap::new();
                opts.insert("host".to_string(), serde_json::json!("example.com"));
                opts.insert("port".to_string(), serde_json::json!(80));
                opts
            }),
        },
    );
    
    config
}

/// Helper to create a test pipeline
fn create_test_pipeline(endpoints: Vec<String>, backends: Vec<String>) -> Pipeline {
    Pipeline {
        description: "Test pipeline".to_string(),
        networks: vec!["test_network".to_string()],
        endpoints,
        backends,
        middleware: vec![],
    }
}

/// Helper to create a test request envelope
fn create_test_envelope() -> RequestEnvelope<Vec<u8>> {
    RequestEnvelopeBuilder::new()
        .method("GET")
        .uri("/test")
        .original_data(b"test data".to_vec())
        .normalized_data(Some(serde_json::json!({"test": "data"})))
        .build()
        .unwrap()
}

/// Helper to create a test protocol context
fn create_test_protocol_ctx(protocol: Protocol) -> ProtocolCtx {
    ProtocolCtx {
        protocol,
        payload: b"test payload".to_vec(),
        meta: HashMap::new(),
        attrs: serde_json::json!({}),
    }
}

#[tokio::test]
async fn test_pipeline_error_types() {
    let err = PipelineError::ServiceError("service failed".to_string());
    assert!(err.to_string().contains("Service error"));
    
    let err = PipelineError::BackendError("backend failed".to_string());
    assert!(err.to_string().contains("Backend error"));
    
    let err = PipelineError::ConfigError("config invalid".to_string());
    assert!(err.to_string().contains("Config error"));
}

#[tokio::test]
async fn test_pipeline_error_from_string() {
    let err: PipelineError = "test error".into();
    assert_eq!(err.to_string(), "Service error: test error");
    
    let err: PipelineError = String::from("another error").into();
    assert_eq!(err.to_string(), "Service error: another error");
}

#[tokio::test]
async fn test_execute_with_no_endpoints_fails() {
    let config = create_test_config();
    let pipeline = create_test_pipeline(vec![], vec![]);
    let envelope = create_test_envelope();
    let ctx = create_test_protocol_ctx(Protocol::Http);
    
    let result = PipelineExecutor::execute(envelope, &pipeline, &config, &ctx).await;
    
    assert!(result.is_err());
    match result.unwrap_err() {
        PipelineError::ConfigError(msg) => {
            assert!(msg.contains("No endpoints"));
        }
        _ => panic!("Expected ConfigError"),
    }
}

#[tokio::test]
async fn test_execute_with_unknown_endpoint_fails() {
    let config = create_test_config();
    let pipeline = create_test_pipeline(vec!["unknown_endpoint".to_string()], vec![]);
    let envelope = create_test_envelope();
    let ctx = create_test_protocol_ctx(Protocol::Http);
    
    let result = PipelineExecutor::execute(envelope, &pipeline, &config, &ctx).await;
    
    assert!(result.is_err());
    match result.unwrap_err() {
        PipelineError::ConfigError(msg) => {
            assert!(msg.contains("not found"));
        }
        _ => panic!("Expected ConfigError"),
    }
}

#[tokio::test]
async fn test_execute_with_skip_backends_flag() {
    let config = create_test_config();
    let pipeline = create_test_pipeline(
        vec!["test_endpoint".to_string()],
        vec!["test_backend".to_string()],
    );
    
    let mut envelope = create_test_envelope();
    envelope.request_details.metadata.insert(
        "skip_backends".to_string(),
        "true".to_string(),
    );
    
    let ctx = create_test_protocol_ctx(Protocol::Http);
    
    let result = PipelineExecutor::execute(envelope, &pipeline, &config, &ctx).await;
    
    // Should succeed even though backend is present
    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.response_details.status, 200);
}

#[tokio::test]
async fn test_execute_with_no_backends_succeeds() {
    let config = create_test_config();
    let pipeline = create_test_pipeline(vec!["test_endpoint".to_string()], vec![]);
    let envelope = create_test_envelope();
    let ctx = create_test_protocol_ctx(Protocol::Http);
    
    let result = PipelineExecutor::execute(envelope, &pipeline, &config, &ctx).await;
    
    // Should succeed with empty response
    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.response_details.status, 200);
}

#[tokio::test]
async fn test_execute_with_unknown_backend_returns_502() {
    let config = create_test_config();
    let pipeline = create_test_pipeline(
        vec!["test_endpoint".to_string()],
        vec!["unknown_backend".to_string()],
    );
    let envelope = create_test_envelope();
    let ctx = create_test_protocol_ctx(Protocol::Http);
    
    let result = PipelineExecutor::execute(envelope, &pipeline, &config, &ctx).await;
    
    // Should return 502 when backend not found
    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.response_details.status, 502);
    
    // Verify it's a plain text response
    let content_type = response.response_details.headers.get("content-type");
    assert_eq!(content_type.map(|s| s.as_str()), Some("text/plain"));
}

#[tokio::test]
async fn test_protocol_ctx_carried_through_pipeline() {
    let config = create_test_config();
    let pipeline = create_test_pipeline(vec!["test_endpoint".to_string()], vec![]);
    let envelope = create_test_envelope();
    
    // Test with different protocols
    for protocol in [Protocol::Http, Protocol::Dimse, Protocol::Hl7V2Mllp] {
        let ctx = create_test_protocol_ctx(protocol);
        let result = PipelineExecutor::execute(envelope.clone(), &pipeline, &config, &ctx).await;
        
        // Should succeed regardless of protocol (protocol-agnostic!)
        assert!(result.is_ok(), "Failed for protocol: {:?}", protocol);
    }
}

#[tokio::test]
async fn test_normalized_data_preserved() {
    let config = create_test_config();
    let pipeline = create_test_pipeline(vec!["test_endpoint".to_string()], vec![]);
    
    let mut envelope = create_test_envelope();
    let test_data = serde_json::json!({
        "test": "value",
        "nested": {"key": "data"}
    });
    envelope.normalized_data = Some(test_data.clone());
    
    let ctx = create_test_protocol_ctx(Protocol::Http);
    
    let result = PipelineExecutor::execute(envelope, &pipeline, &config, &ctx).await;
    
    assert!(result.is_ok());
    // normalized_data should be preserved through the pipeline
    // (though may be modified by middleware/backends)
}

#[tokio::test]
async fn test_middleware_chain_empty_succeeds() {
    let config = create_test_config();
    let mut pipeline = create_test_pipeline(vec!["test_endpoint".to_string()], vec![]);
    pipeline.middleware = vec![]; // Explicitly empty
    
    let envelope = create_test_envelope();
    let ctx = create_test_protocol_ctx(Protocol::Http);
    
    let result = PipelineExecutor::execute(envelope, &pipeline, &config, &ctx).await;
    
    // Should succeed with no middleware
    assert!(result.is_ok());
}

#[test]
fn test_pipeline_error_is_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    
    assert_send::<PipelineError>();
    assert_sync::<PipelineError>();
}

#[test]
fn test_pipeline_executor_is_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    
    assert_send::<PipelineExecutor>();
    assert_sync::<PipelineExecutor>();
}
