use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;

/// A simple test middleware that passes through but annotates the normalized_data
pub struct PassthruMiddleware;

impl Default for PassthruMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl PassthruMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Middleware for PassthruMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Ensure normalized_data is an object and set a marker
        let mut obj = envelope
            .normalized_data
            .clone()
            .unwrap_or(serde_json::json!({}));
        if let Some(map) = obj.as_object_mut() {
            map.insert("mw_left".to_string(), serde_json::json!(true));
        }
        envelope.normalized_data = Some(obj);
        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        // Passthrough - optionally annotate for debugging
        if let Some(mut obj) = envelope.normalized_data.clone() {
            if let Some(map) = obj.as_object_mut() {
                map.insert("mw_right".to_string(), serde_json::json!(true));
            }
            envelope.normalized_data = Some(obj);
        }
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
            content_metadata: None,
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
    async fn test_left_adds_marker_to_normalized_data() {
        let middleware = PassthruMiddleware::new();
        let original_data = json!({"key": "value"});
        let normalized_data = json!({"existing": "data"});
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data,
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        let data = result.normalized_data.unwrap();
        assert_eq!(data["mw_left"], json!(true));
        assert_eq!(data["existing"], json!("data"));
    }

    #[tokio::test]
    async fn test_left_creates_object_when_missing() {
        let middleware = PassthruMiddleware::new();
        let original_data = json!({"key": "value"});
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data,
            normalized_data: None,
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        let data = result.normalized_data.unwrap();
        assert_eq!(data["mw_left"], json!(true));
    }

    #[tokio::test]
    async fn test_left_handles_non_object_gracefully() {
        let middleware = PassthruMiddleware::new();
        let original_data = json!({"key": "value"});
        let normalized_data = json!("not an object");
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data,
            normalized_data: Some(normalized_data.clone()),
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope).await.unwrap();

        // Should still succeed but not add marker since it's not an object
        assert!(result.normalized_data.is_some());
        let data = result.normalized_data.unwrap();
        assert_eq!(data, normalized_data);
    }

    #[tokio::test]
    async fn test_right_adds_marker_to_normalized_data() {
        let middleware = PassthruMiddleware::new();
        let normalized_data = json!({"response": "data"});
        let request_details = create_test_request_details();
        let response_details = create_test_response_details();

        let envelope = ResponseEnvelope {
            request_details,
            response_details,
            original_data: json!({"original": "response"}),
            normalized_data: Some(normalized_data),
            normalized_snapshot: None,
        };

        let result = middleware.right(envelope).await.unwrap();

        assert!(result.normalized_data.is_some());
        let data = result.normalized_data.unwrap();
        assert_eq!(data["mw_right"], json!(true));
        assert_eq!(data["response"], json!("data"));
    }

    #[tokio::test]
    async fn test_right_handles_missing_normalized_data() {
        let middleware = PassthruMiddleware::new();
        let request_details = create_test_request_details();
        let response_details = create_test_response_details();

        let envelope = ResponseEnvelope {
            request_details,
            response_details,
            original_data: json!({"original": "response"}),
            normalized_data: None,
            normalized_snapshot: None,
        };

        let result = middleware.right(envelope).await.unwrap();

        // Should succeed with no changes since normalized_data is None
        assert!(result.normalized_data.is_none());
    }

    #[tokio::test]
    async fn test_right_handles_non_object_gracefully() {
        let middleware = PassthruMiddleware::new();
        let normalized_data = json!([1, 2, 3]);
        let request_details = create_test_request_details();
        let response_details = create_test_response_details();

        let envelope = ResponseEnvelope {
            request_details,
            response_details,
            original_data: json!({"original": "response"}),
            normalized_data: Some(normalized_data.clone()),
            normalized_snapshot: None,
        };

        let result = middleware.right(envelope).await.unwrap();

        // Should still succeed but not add marker since it's not an object
        assert!(result.normalized_data.is_some());
        let data = result.normalized_data.unwrap();
        assert_eq!(data, normalized_data);
    }

    #[tokio::test]
    async fn test_left_and_right_markers() {
        let middleware = PassthruMiddleware::new();
        let original_data = json!({"key": "value"});
        let request_details = create_test_request_details();
        let response_details = create_test_response_details();

        // Left pass
        let request_envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details.clone(),
            target_details: None,
            original_data,
            normalized_data: Some(json!({})),
            normalized_snapshot: None,
        };

        let after_left = middleware.left(request_envelope).await.unwrap();
        assert_eq!(
            after_left.normalized_data.as_ref().unwrap()["mw_left"],
            json!(true)
        );

        // Right pass
        let response_envelope = ResponseEnvelope {
            request_details,
            response_details,
            original_data: json!({}),
            normalized_data: after_left.normalized_data.clone(),
            normalized_snapshot: None,
        };

        let after_right = middleware.right(response_envelope).await.unwrap();
        let final_data = after_right.normalized_data.unwrap();
        assert_eq!(final_data["mw_left"], json!(true));
        assert_eq!(final_data["mw_right"], json!(true));
    }

    #[tokio::test]
    async fn test_default_constructor() {
        let middleware1 = PassthruMiddleware::default();
        let middleware2 = PassthruMiddleware::new();

        let original_data = json!({"test": "data"});
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data,
            normalized_data: Some(json!({})),
            normalized_snapshot: None,
        };

        let result1 = middleware1.left(envelope.clone()).await.unwrap();
        let result2 = middleware2.left(envelope).await.unwrap();

        assert_eq!(result1.normalized_data, result2.normalized_data);
    }
}
