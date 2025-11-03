use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct AuraboxConnectConfig {
    pub enabled: bool,
    pub fallback_timeout_ms: u64,
}

pub struct AuraboxConnectMiddleware {
    #[allow(dead_code)]
    config: AuraboxConnectConfig,
}

pub fn parse_config(
    options: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<AuraboxConnectConfig, String> {
    let enabled = options
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let fallback_timeout_ms = options
        .get("fallback_timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000);

    Ok(AuraboxConnectConfig {
        enabled,
        fallback_timeout_ms,
    })
}

impl AuraboxConnectMiddleware {
    pub fn new(config: AuraboxConnectConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Middleware for AuraboxConnectMiddleware {
    async fn left(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        if !self.config.enabled {
            // If the middleware is disabled, log and skip further handling
            tracing::info!("AuraboxConnectMiddleware is disabled, skipping middleware logic.");
            return Ok(envelope);
        }

        // Simulate some logic based on `fallback_timeout_ms` (e.g., logging or conditional behavior)
        tracing::info!(
            "AuraboxConnectMiddleware handling request with fallback timeout: {} ms",
            self.config.fallback_timeout_ms
        );

        // For now, just pass through the envelope
        // In a real implementation, you might modify the envelope based on connection logic
        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        if !self.config.enabled {
            tracing::debug!("AuraboxConnectMiddleware is disabled for right processing.");
            return Ok(envelope);
        }

        tracing::debug!("AuraboxConnectMiddleware processing response (right) - passthrough");
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

    #[test]
    fn test_parse_config_with_all_options() {
        let mut options = HashMap::new();
        options.insert("enabled".to_string(), json!(true));
        options.insert("fallback_timeout_ms".to_string(), json!(3000));

        let config = parse_config(&options).unwrap();

        assert_eq!(config.enabled, true);
        assert_eq!(config.fallback_timeout_ms, 3000);
    }

    #[test]
    fn test_parse_config_with_defaults() {
        let options = HashMap::new();

        let config = parse_config(&options).unwrap();

        // Defaults: enabled=true, fallback_timeout_ms=5000
        assert_eq!(config.enabled, true);
        assert_eq!(config.fallback_timeout_ms, 5000);
    }

    #[test]
    fn test_parse_config_disabled() {
        let mut options = HashMap::new();
        options.insert("enabled".to_string(), json!(false));

        let config = parse_config(&options).unwrap();

        assert_eq!(config.enabled, false);
        assert_eq!(config.fallback_timeout_ms, 5000); // Still uses default
    }

    #[test]
    fn test_parse_config_custom_timeout() {
        let mut options = HashMap::new();
        options.insert("fallback_timeout_ms".to_string(), json!(10000));

        let config = parse_config(&options).unwrap();

        assert_eq!(config.enabled, true);
        assert_eq!(config.fallback_timeout_ms, 10000);
    }

    #[test]
    fn test_parse_config_invalid_type_for_enabled() {
        let mut options = HashMap::new();
        options.insert("enabled".to_string(), json!("not a bool"));

        let config = parse_config(&options).unwrap();

        // Should fall back to default (true) when type is wrong
        assert_eq!(config.enabled, true);
    }

    #[test]
    fn test_parse_config_invalid_type_for_timeout() {
        let mut options = HashMap::new();
        options.insert("fallback_timeout_ms".to_string(), json!("not a number"));

        let config = parse_config(&options).unwrap();

        // Should fall back to default (5000) when type is wrong
        assert_eq!(config.fallback_timeout_ms, 5000);
    }

    #[tokio::test]
    async fn test_middleware_enabled_left() {
        let config = AuraboxConnectConfig {
            enabled: true,
            fallback_timeout_ms: 3000,
        };
        let middleware = AuraboxConnectMiddleware::new(config);
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: json!({"test": "data"}),
            normalized_data: Some(json!({"key": "value"})),
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope.clone()).await.unwrap();

        // Should pass through unchanged
        assert_eq!(result.normalized_data, envelope.normalized_data);
    }

    #[tokio::test]
    async fn test_middleware_disabled_left() {
        let config = AuraboxConnectConfig {
            enabled: false,
            fallback_timeout_ms: 3000,
        };
        let middleware = AuraboxConnectMiddleware::new(config);
        let request_details = create_test_request_details();

        let envelope = RequestEnvelope {
            request_details: request_details.clone(),
            backend_request_details: request_details,
            target_details: None,
            original_data: json!({"test": "data"}),
            normalized_data: Some(json!({"key": "value"})),
            normalized_snapshot: None,
        };

        let result = middleware.left(envelope.clone()).await.unwrap();

        // Should pass through unchanged when disabled
        assert_eq!(result.normalized_data, envelope.normalized_data);
    }

    #[tokio::test]
    async fn test_middleware_enabled_right() {
        let config = AuraboxConnectConfig {
            enabled: true,
            fallback_timeout_ms: 3000,
        };
        let middleware = AuraboxConnectMiddleware::new(config);
        let request_details = create_test_request_details();
        let response_details = create_test_response_details();

        let envelope = ResponseEnvelope {
            request_details,
            response_details,
            original_data: json!({"response": "data"}),
            normalized_data: Some(json!({"result": "success"})),
            normalized_snapshot: None,
        };

        let result = middleware.right(envelope.clone()).await.unwrap();

        // Should pass through unchanged
        assert_eq!(result.normalized_data, envelope.normalized_data);
    }

    #[tokio::test]
    async fn test_middleware_disabled_right() {
        let config = AuraboxConnectConfig {
            enabled: false,
            fallback_timeout_ms: 3000,
        };
        let middleware = AuraboxConnectMiddleware::new(config);
        let request_details = create_test_request_details();
        let response_details = create_test_response_details();

        let envelope = ResponseEnvelope {
            request_details,
            response_details,
            original_data: json!({"response": "data"}),
            normalized_data: Some(json!({"result": "success"})),
            normalized_snapshot: None,
        };

        let result = middleware.right(envelope.clone()).await.unwrap();

        // Should pass through unchanged when disabled
        assert_eq!(result.normalized_data, envelope.normalized_data);
    }

    #[test]
    fn test_config_serialization() {
        let config = AuraboxConnectConfig {
            enabled: true,
            fallback_timeout_ms: 7500,
        };

        // Test that config can be serialized and deserialized
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: AuraboxConnectConfig = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.enabled, config.enabled);
        assert_eq!(deserialized.fallback_timeout_ms, config.fallback_timeout_ms);
    }
}
