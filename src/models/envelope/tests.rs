#![cfg(test)]

use crate::models::envelope::envelope::{RequestDetails, RequestEnvelope, RequestEnvelopeBuilder};
use std::collections::HashMap;

#[test]
fn test_create_envelope() {
    let request_details = RequestDetails {
        method: "POST".to_string(),
        uri: "/example-path".to_string(),
        headers: {
            let mut headers = HashMap::new();
            headers.insert("Content-Type".to_string(), "application/json".to_string());
            headers
        },
        cookies: HashMap::new(),
        query_params: HashMap::new(),
        cache_status: None,
        metadata: HashMap::new(),
    };

    let original_data = vec!["example", "data"];
    let envelope = RequestEnvelope::new(request_details.clone(), original_data);

    assert_eq!(envelope.request_details.method, request_details.method);
    assert!(envelope.normalized_data.is_some());
}

// Builder tests

#[test]
fn test_builder_minimal() {
    let envelope = RequestEnvelopeBuilder::new()
        .method("GET")
        .uri("/test")
        .original_data(b"test data".to_vec())
        .build()
        .unwrap();

    assert_eq!(envelope.request_details.method, "GET");
    assert_eq!(envelope.request_details.uri, "/test");
    assert_eq!(envelope.original_data, b"test data".to_vec());
    assert!(envelope.request_details.headers.is_empty());
    assert!(envelope.request_details.cookies.is_empty());
    assert!(envelope.request_details.query_params.is_empty());
    assert_eq!(envelope.request_details.cache_status, None);
    assert!(envelope.request_details.metadata.is_empty());
}

#[test]
fn test_builder_with_minimal() {
    let envelope = RequestEnvelopeBuilder::with_minimal("POST", "/api/test", vec![1, 2, 3])
        .build()
        .unwrap();

    assert_eq!(envelope.request_details.method, "POST");
    assert_eq!(envelope.request_details.uri, "/api/test");
    assert_eq!(envelope.original_data, vec![1, 2, 3]);
}

#[test]
fn test_builder_auto_normalizes_data() {
    let data = serde_json::json!({"key": "value", "number": 42});
    let envelope = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("/json")
        .original_data(data.clone())
        .build()
        .unwrap();

    assert!(envelope.normalized_data.is_some());
    let normalized = envelope.normalized_data.unwrap();
    assert_eq!(normalized["key"], "value");
    assert_eq!(normalized["number"], 42);
}

#[test]
fn test_builder_auto_clones_backend_request_details() {
    let envelope: RequestEnvelope<Vec<u8>> = RequestEnvelopeBuilder::new()
        .method("GET")
        .uri("/test")
        .header("X-Custom", "test")
        .original_data(vec![])
        .build()
        .unwrap();

    // backend_request_details should be cloned from request_details
    assert_eq!(
        envelope.request_details.method,
        envelope.backend_request_details.method
    );
    assert_eq!(
        envelope.request_details.uri,
        envelope.backend_request_details.uri
    );
    assert_eq!(
        envelope.request_details.headers,
        envelope.backend_request_details.headers
    );
}

#[test]
fn test_builder_with_headers() {
    let envelope: RequestEnvelope<Vec<u8>> = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("/api")
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer token")
        .original_data(vec![])
        .build()
        .unwrap();

    assert_eq!(
        envelope.request_details.headers.get("Content-Type"),
        Some(&"application/json".to_string())
    );
    assert_eq!(
        envelope.request_details.headers.get("Authorization"),
        Some(&"Bearer token".to_string())
    );
}

#[test]
fn test_builder_with_metadata() {
    let envelope: RequestEnvelope<Vec<u8>> = RequestEnvelopeBuilder::new()
        .method("GET")
        .uri("/test")
        .metadata_entry("request_id", "123")
        .metadata_entry("user_id", "456")
        .original_data(vec![])
        .build()
        .unwrap();

    assert_eq!(
        envelope.request_details.metadata.get("request_id"),
        Some(&"123".to_string())
    );
    assert_eq!(
        envelope.request_details.metadata.get("user_id"),
        Some(&"456".to_string())
    );
}

#[test]
fn test_builder_with_query_params() {
    let envelope: RequestEnvelope<Vec<u8>> = RequestEnvelopeBuilder::new()
        .method("GET")
        .uri("/search")
        .query_param("q", vec!["test".to_string()])
        .query_param("limit", vec!["10".to_string()])
        .original_data(vec![])
        .build()
        .unwrap();

    assert_eq!(
        envelope.request_details.query_params.get("q"),
        Some(&vec!["test".to_string()])
    );
    assert_eq!(
        envelope.request_details.query_params.get("limit"),
        Some(&vec!["10".to_string()])
    );
}

#[test]
fn test_builder_with_cookies() {
    let envelope: RequestEnvelope<Vec<u8>> = RequestEnvelopeBuilder::new()
        .method("GET")
        .uri("/test")
        .cookie("session", "abc123")
        .cookie("user", "john")
        .original_data(vec![])
        .build()
        .unwrap();

    assert_eq!(
        envelope.request_details.cookies.get("session"),
        Some(&"abc123".to_string())
    );
    assert_eq!(
        envelope.request_details.cookies.get("user"),
        Some(&"john".to_string())
    );
}

#[test]
fn test_builder_from_request_details() {
    let request_details = RequestDetails {
        method: "PUT".to_string(),
        uri: "/api/resource".to_string(),
        headers: {
            let mut h = HashMap::new();
            h.insert("X-Test".to_string(), "value".to_string());
            h
        },
        cookies: HashMap::new(),
        query_params: HashMap::new(),
        cache_status: Some("HIT".to_string()),
        metadata: HashMap::new(),
    };

    let envelope = RequestEnvelopeBuilder::from_request_details(request_details.clone())
        .original_data(vec![1, 2, 3])
        .build()
        .unwrap();

    assert_eq!(envelope.request_details.method, "PUT");
    assert_eq!(envelope.request_details.uri, "/api/resource");
    assert_eq!(
        envelope.request_details.headers.get("X-Test"),
        Some(&"value".to_string())
    );
    assert_eq!(
        envelope.request_details.cache_status,
        Some("HIT".to_string())
    );
}

#[test]
fn test_builder_explicit_normalized_data() {
    let custom_normalized = serde_json::json!({"custom": "data"});
    let envelope = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("/test")
        .original_data(vec![1, 2, 3])
        .normalized_data(Some(custom_normalized.clone()))
        .build()
        .unwrap();

    assert_eq!(envelope.normalized_data, Some(custom_normalized));
}

#[test]
fn test_builder_explicit_backend_request_details() {
    let backend_details = RequestDetails {
        method: "POST".to_string(),
        uri: "/backend/api".to_string(),
        headers: HashMap::new(),
        cookies: HashMap::new(),
        query_params: HashMap::new(),
        cache_status: None,
        metadata: HashMap::new(),
    };

    let envelope: RequestEnvelope<Vec<u8>> = RequestEnvelopeBuilder::new()
        .method("GET")
        .uri("/frontend/api")
        .backend_request_details(backend_details.clone())
        .original_data(vec![])
        .build()
        .unwrap();

    // Request details should be different from backend_request_details
    assert_eq!(envelope.request_details.method, "GET");
    assert_eq!(envelope.backend_request_details.method, "POST");
    assert_eq!(envelope.request_details.uri, "/frontend/api");
    assert_eq!(envelope.backend_request_details.uri, "/backend/api");
}

#[test]
fn test_builder_with_normalized_snapshot() {
    let snapshot = serde_json::json!({"snapshot": "data"});
    let envelope: RequestEnvelope<Vec<u8>> = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("/test")
        .original_data(vec![])
        .normalized_snapshot(Some(snapshot.clone()))
        .build()
        .unwrap();

    assert_eq!(envelope.normalized_snapshot, Some(snapshot));
}

#[test]
fn test_builder_missing_method_fails() {
    let result = RequestEnvelopeBuilder::<Vec<u8>>::new()
        .uri("/test")
        .original_data(vec![])
        .build();

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("method"));
}

#[test]
fn test_builder_missing_uri_fails() {
    let result = RequestEnvelopeBuilder::<Vec<u8>>::new()
        .method("GET")
        .original_data(vec![])
        .build();

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("uri"));
}

#[test]
fn test_builder_missing_original_data_fails() {
    let result = RequestEnvelopeBuilder::<Vec<u8>>::new()
        .method("GET")
        .uri("/test")
        .build();

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("original_data"));
}

#[test]
fn test_builder_with_different_types() {
    // Test with Vec<u8>
    let envelope1 = RequestEnvelopeBuilder::new()
        .method("GET")
        .uri("/test")
        .original_data(b"bytes".to_vec())
        .build()
        .unwrap();
    assert_eq!(envelope1.original_data, b"bytes".to_vec());

    // Test with serde_json::Value
    let json_data = serde_json::json!({"key": "value"});
    let envelope2 = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("/json")
        .original_data(json_data.clone())
        .build()
        .unwrap();
    assert_eq!(envelope2.original_data, json_data);
}

#[test]
fn test_builder_chaining() {
    // Test that all builder methods return self for chaining
    let envelope = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("/api/test")
        .header("Content-Type", "application/json")
        .cookie("session", "xyz")
        .query_param("id", vec!["123".to_string()])
        .metadata_entry("request_id", "abc")
        .cache_status(Some("MISS".to_string()))
        .original_data(serde_json::json!({"test": true}))
        .build()
        .unwrap();

    assert_eq!(envelope.request_details.method, "POST");
    assert_eq!(envelope.request_details.uri, "/api/test");
    assert!(envelope.request_details.headers.contains_key("Content-Type"));
    assert!(envelope.request_details.cookies.contains_key("session"));
    assert!(envelope.request_details.query_params.contains_key("id"));
    assert!(envelope.request_details.metadata.contains_key("request_id"));
    assert_eq!(
        envelope.request_details.cache_status,
        Some("MISS".to_string())
    );
}

#[test]
fn test_request_envelope_builder_via_envelope() {
    // Test RequestEnvelope::builder() method
    let envelope = RequestEnvelope::<Vec<u8>>::builder()
        .method("GET")
        .uri("/via-envelope")
        .original_data(vec![1, 2, 3])
        .build()
        .unwrap();

    assert_eq!(envelope.request_details.method, "GET");
    assert_eq!(envelope.request_details.uri, "/via-envelope");
}
