use serde::{Deserialize, Serialize};
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct AuraboxConnectConfig {
    pub enabled: bool,
    pub fallback_timeout_ms: u64,
}

pub struct AuraboxConnectMiddleware {
    #[allow(dead_code)]
    config: AuraboxConnectConfig,
}

pub fn parse_config(options: &std::collections::HashMap<String, serde_json::Value>) -> Result<AuraboxConnectConfig, String> {
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
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        if !self.config.enabled {
            tracing::info!("AuraboxConnectMiddleware is disabled for right processing.");
            return Ok(envelope);
        }

        tracing::info!("AuraboxConnectMiddleware processing response (right)");
        Ok(envelope)
    }
}