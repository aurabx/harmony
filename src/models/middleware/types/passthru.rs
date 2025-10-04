use crate::models::envelope::envelope::Envelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;

/// A simple test middleware that passes through but annotates the normalized_data
pub struct PassthruMiddleware;

impl PassthruMiddleware {
    pub fn new() -> Self { Self }
}

#[async_trait::async_trait]
impl Middleware for PassthruMiddleware {
    async fn left(
        &self,
        mut envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        // Ensure normalized_data is an object and set a marker
        let mut obj = envelope.normalized_data.clone().unwrap_or(serde_json::json!({}));
        if let Some(map) = obj.as_object_mut() {
            map.insert("mw_left".to_string(), serde_json::json!(true));
        }
        envelope.normalized_data = Some(obj);
        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        let mut obj = envelope.normalized_data.clone().unwrap_or(serde_json::json!({}));
        if let Some(map) = obj.as_object_mut() {
            map.insert("mw_right".to_string(), serde_json::json!(true));
        }
        envelope.normalized_data = Some(obj);
        Ok(envelope)
    }
}