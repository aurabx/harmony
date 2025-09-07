pub mod config;
pub mod jwtauth;
pub mod auth;
pub mod connect;
pub(crate) mod types;
pub mod chain;

use axum::{
    Router,
    response::Response,
    http::Request,
    body::Body,
};
use async_trait::async_trait;
use axum::middleware::Next as NextMiddleware;
use std::sync::Arc;
use tower::{service_fn};
use tower::util::BoxCloneService;
use crate::models::envelope::envelope::Envelope;
use crate::models::middleware::auth::AuthSidecarMiddleware;
use crate::models::middleware::chain::MiddlewareChain;
use crate::models::middleware::config::*;
use crate::models::middleware::connect::AuraboxConnectMiddleware;
use crate::models::middleware::jwtauth::JwtAuthMiddleware;

use crate::models::middleware::types::{Next, Error};


#[async_trait]
pub trait Middleware: Send + Sync {
    /// Transform the request Envelope on its way to the backend.
    async fn transform_request(
        &self,
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, crate::models::middleware::types::Error>;

    /// Transform the response Envelope on its way back from the backend.
    async fn transform_response(
        &self,
        envelope: Envelope<serde_json::Value>,
    ) -> Result<Envelope<serde_json::Value>, crate::models::middleware::types::Error>;
}


pub struct MiddlewareState {
    #[allow(dead_code)]
    config: MiddlewareConfig,
}


pub fn build_middleware_stack(
    router: Router,
    middleware_list: &[String],
    config: MiddlewareConfig,
) -> Router {
    if middleware_list.is_empty() {
        return router;
    }

    let middleware_chain = MiddlewareChain::new(middleware_list, &config);

    router.layer(axum::middleware::from_fn(move |req: Request<Body>, next: NextMiddleware| {
        let middleware = middleware_chain.clone();
        async move {
            // Execute custom middleware
            match middleware.left(req).await {
                Ok(req) => {
                    // Pass the successful request to the next handler in the middleware
                    next.run(req).await
                }
                Err(err) => {
                    // Log the error
                    let error_message = format!("Middleware middleware error: {}", err);
                    tracing::error!("{}", error_message);

                    // Return an error response
                    Response::builder()
                        .status(500)
                        .body(Body::from(error_message))
                        .unwrap()
                }
            }
        }
    }))
}

pub async fn process_request_through_chain(
    envelope: Envelope<serde_json::Value>,
    middlewares: &[Box<dyn Middleware>],
) -> Result<Envelope<serde_json::Value>, crate::models::middleware::types::Error> {
    let mut transformed_envelope = envelope;
    for middleware in middlewares {
        transformed_envelope = middleware
            .transform_request(transformed_envelope)
            .await?;
    }
    Ok(transformed_envelope)
}

pub async fn process_response_through_chain(
    envelope: Envelope<serde_json::Value>,
    middlewares: &[Box<dyn Middleware>],
) -> Result<Envelope<serde_json::Value>, crate::models::middleware::types::Error> {
    let mut transformed_envelope = envelope;
    for middleware in middlewares.iter().rev() {
        transformed_envelope = middleware
            .transform_response(transformed_envelope)
            .await?;
    }
    Ok(transformed_envelope)
}
