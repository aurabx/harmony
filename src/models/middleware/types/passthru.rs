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
