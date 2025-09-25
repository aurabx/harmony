use std::sync::Arc;
use std::collections::HashMap;
use crate::models::middleware::middleware::{Middleware, resolve_middleware};
use crate::models::envelope::envelope::Envelope;
use crate::utils::Error;
use serde_json::Value;

/// Struct representing a chain of middleware
#[derive(Clone)]
pub struct MiddlewareChain {
    middlewares: Arc<Vec<Box<dyn Middleware>>>,
}

impl MiddlewareChain {
    /// Create a new `MiddlewareChain` from middleware instances with their configurations
    pub fn new(middleware_instances: &[(String, HashMap<String, Value>)]) -> Self {
        let middlewares = middleware_instances
            .iter()
            .filter_map(|(middleware_type, options)| {
                match resolve_middleware(middleware_type, options) {
                    Ok(middleware) => Some(middleware),
                    Err(err) => {
                        tracing::error!("Failed to resolve middleware '{}': {}", middleware_type, err);
                        None
                    }
                }
            })
            .collect();

        Self {
            middlewares: Arc::new(middlewares),
        }
    }

    /// Create middleware chain from a simplified list (for backward compatibility)
    pub fn from_simple_list(middleware_list: &[String]) -> Self {
        let middleware_instances: Vec<(String, HashMap<String, Value>)> = middleware_list
            .iter()
            .map(|name| (name.clone(), HashMap::new()))
            .collect();
        
        Self::new(&middleware_instances)
    }

    /// Processes the incoming envelope through the "left" middleware chain.
    pub async fn left(
        &self,
        mut envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        for middleware in self.middlewares.iter() {
            // Pass the envelope through the middleware
            envelope = middleware.left(envelope).await?;
        }
        Ok(envelope)
    }

    /// Processes the outgoing envelope through the "right" middleware chain.
    pub async fn right(
        &self,
        mut envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, Error> {
        // Process middleware in reverse order for right-side processing
        for middleware in self.middlewares.iter().rev() {
            // Pass the envelope through the middleware
            envelope = middleware.right(envelope).await?;
        }
        Ok(envelope)
    }
}