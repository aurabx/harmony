use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;

/// JSON Extractor middleware
///
/// Responsibility:
/// - If normalized_data is not set, copy original_data (serde_json::Value) into normalized_data.
/// - This assumes the conversion layer already attempted to parse bytes as JSON into original_data.
/// - Runs typically after authentication middleware.
pub struct JsonExtractorMiddleware;

impl Default for JsonExtractorMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonExtractorMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Middleware for JsonExtractorMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Only set normalized_data if missing
        if envelope.normalized_data.is_none() {
            envelope.normalized_data = Some(envelope.original_data.clone());
        }
        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        // JSON extraction not needed on response side (dispatcher handles it)
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{RequestDetails, ResponseDetails};
    use serde_json::json;
    use std::collections::HashMap;

    fn create_test_request_details() -> RequestDetails {
        RequestDetails {
            method: "POST".to_string(),
            uri: "/test".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata: HashMap::new(),
        }
    }

    fn create_test_response_details() -> ResponseDetails {
        ResponseDetails {
            status: 200,
            headers: HashMap::new(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_left_copies_original_to_normalized_when_missing() {
        let middleware = JsonExtractorMiddleware::new();
        let original_data = json!({"key": "value"});
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: original_data.clone(),
            normalized_data: None,
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        assert_eq!(result.normalized_data.unwrap(), original_data);
    }

    #[tokio::test]
    async fn test_left_preserves_existing_normalized_data() {
        let middleware = JsonExtractorMiddleware::new();
        let original_data = json!({"key": "original"});
        let normalized_data = json!({"key": "normalized"});
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: original_data.clone(),
            normalized_data: Some(normalized_data.clone()),
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        assert_eq!(result.normalized_data.unwrap(), normalized_data);
        assert_ne!(result.original_data, normalized_data);
    }

    #[tokio::test]
    async fn test_left_with_json_array() {
        let middleware = JsonExtractorMiddleware::new();
        let original_data = json!([1, 2, 3, 4]);
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: original_data.clone(),
            normalized_data: None,
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        assert_eq!(result.normalized_data.unwrap(), original_data);
    }

    #[tokio::test]
    async fn test_left_with_json_primitive() {
        let middleware = JsonExtractorMiddleware::new();
        let original_data = json!("simple string");
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: original_data.clone(),
            normalized_data: None,
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        assert_eq!(result.normalized_data.unwrap(), original_data);
    }

    #[tokio::test]
    async fn test_left_with_json_null() {
        let middleware = JsonExtractorMiddleware::new();
        let original_data = json!(null);
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: original_data.clone(),
            normalized_data: None,
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        assert_eq!(result.normalized_data.unwrap(), original_data);
    }

    #[tokio::test]
    async fn test_left_with_complex_json() {
        let middleware = JsonExtractorMiddleware::new();
        let original_data = json!({
            "nested": {
                "array": [1, 2, {"key": "value"}],
                "bool": true,
                "null": null
            }
        });
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: original_data.clone(),
            normalized_data: None,
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        assert_eq!(result.normalized_data.unwrap(), original_data);
    }

    #[tokio::test]
    async fn test_right_passes_through_unchanged() {
        let middleware = JsonExtractorMiddleware::new();
        let normalized_data = json!({"normalized": "response"});
        let request_details = create_test_request_details();
        let response_details = create_test_response_details();

        let envelope = ResponseEnvelope {
            request_details,
            response_details,
            original_data: json!({"response": "data"}),
            normalized_data: Some(normalized_data.clone()),
            normalized_snapshot: None,
        };

        let result = middleware.right(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        assert_eq!(result.normalized_data.unwrap(), normalized_data);
    }

    #[tokio::test]
    async fn test_default_constructor() {
        let middleware1 = JsonExtractorMiddleware::default();
        let middleware2 = JsonExtractorMiddleware::new();

        // Both constructors should create equivalent instances
        let original_data = json!({"test": "data"});
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: original_data.clone(),
            normalized_data: None,
            normalized_snapshot: None,
        };

        let result1 = middleware1.left(envelope.clone()).await.unwrap();
        let result2 = middleware2.left(envelope).await.unwrap();

        assert_eq!(result1.normalized_data, result2.normalized_data);
    }
}
