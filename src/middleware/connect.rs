use serde::Deserialize;
use tower::Service;
use axum::{
    response::Response,
    http::Request,
    body::Body,
};
use crate::middleware::{Middleware, Next, Error};

#[derive(Debug, Deserialize, Clone)]
pub struct AuraboxConnectConfig {
    pub enabled: bool,
    pub fallback_timeout_ms: u64,
}

pub struct AuraboxConnectMiddleware {
    #[allow(dead_code)]
    config: AuraboxConnectConfig,
}

impl AuraboxConnectMiddleware {
    pub fn new(config: AuraboxConnectConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Middleware for AuraboxConnectMiddleware {
    async fn handle(
        &self,
        request: Request<Body>,
        mut next: Next<Body>,
    ) -> Result<Response, Error> {
        if !self.config.enabled {
            // If the middleware is disabled, log and skip further handling
            tracing::info!("AuraboxConnectMiddleware is disabled, skipping middleware logic.");
            return next.call(request).await;
        }

        // Simulate some logic based on `fallback_timeout_ms` (e.g., logging or conditional behavior)
        tracing::info!(
            "AuraboxConnectMiddleware handling request with fallback timeout: {} ms",
            self.config.fallback_timeout_ms
        );

        // Proceed with the next middleware or handler
        next.call(request).await.map_err(|err| {
            let error_message = format!("AuraboxConnectMiddleware encountered an error: {}", err);
            tracing::error!("{}", error_message);
            err
        })
    }
}
