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
