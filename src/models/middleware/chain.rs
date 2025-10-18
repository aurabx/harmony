use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use std::sync::Arc;

/// Struct representing a chain of middleware
#[derive(Clone)]
pub struct MiddlewareChain {
    middlewares: Arc<Vec<Box<dyn Middleware>>>,
}

impl MiddlewareChain {
    /// Create a new `MiddlewareChain` from pre-built middleware instances
    pub fn new(middlewares: impl IntoIterator<Item = Box<dyn Middleware>>) -> Self {
        Self {
            middlewares: Arc::new(middlewares.into_iter().collect()),
        }
    }

    /// Processes the incoming envelope through the "left" middleware chain.
    pub async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        for middleware in self.middlewares.iter() {
            // Pass the envelope through the middleware
            envelope = middleware.left(envelope).await?;
        }
        Ok(envelope)
    }

    /// Processes the response envelope through the "right" middleware chain.
    pub async fn right(
        &self,
        mut envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        // Process middleware in reverse order for right-side processing
        for middleware in self.middlewares.iter().rev() {
            // Pass the envelope through the middleware
            envelope = middleware.right(envelope).await?;
        }
        Ok(envelope)
    }
}
