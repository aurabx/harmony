pub mod config;

pub mod jwtauth;
pub mod auth;
pub mod connect;
mod types;

use axum::{
    Router,
    response::Response,
    http::Request,
    body::Body,
};

use axum::middleware::Next as NextMiddleware;

use std::sync::Arc;
use tower::{service_fn};
use tower::util::BoxCloneService;

use crate::middleware::auth::AuthSidecarMiddleware;
use crate::middleware::config::*;
use crate::middleware::connect::AuraboxConnectMiddleware;
use crate::middleware::jwtauth::JwtAuthMiddleware;

use crate::middleware::types::{Next, Error};


#[async_trait::async_trait]
pub trait Middleware: Send + Sync {
    async fn handle(
        &self,
        request: Request<Body>,
        next: Next<Body>,
    ) -> Result<Response, Error>;
}

pub struct MiddlewareState {
    #[allow(dead_code)]
    config: MiddlewareConfig,
}

#[derive(Clone)]
pub struct MiddlewareChain {
    middlewares: Arc<Vec<Box<dyn Middleware>>>,
}

impl MiddlewareChain {
    pub fn new(
        middleware_list: &[String],
        config: &MiddlewareConfig
    ) -> Self {
        let middlewares = middleware_list.iter()
            .filter_map(|name| Self::create_middleware(name, config))
            .collect();

        Self {
            middlewares: Arc::new(middlewares)
        }
    }

    fn create_middleware(
        name: &str,
        config: &MiddlewareConfig
    ) -> Option<Box<dyn Middleware>> {
        match name {
            "jwt_auth" => {
                config.jwt_auth.as_ref().map(|cfg| {
                    Box::new(JwtAuthMiddleware::new(cfg.clone())) as Box<dyn Middleware>
                })
            },
            "auth_sidecar" => {
                config.auth_sidecar.as_ref().map(|cfg| {
                    Box::new(AuthSidecarMiddleware::new(cfg.clone())) as Box<dyn Middleware>
                })
            },
            "aurabox_connect" => {
                config.aurabox_connect.as_ref().map(|cfg| {
                    Box::new(AuraboxConnectMiddleware::new(cfg.clone())) as Box<dyn Middleware>
                })
            },
            _ => {
                tracing::warn!("Unknown middleware: {}", name);
                None
            }
        }
    }

    pub async fn execute(&self, mut request: Request<Body>) -> Result<Request<Body>, Error> {
        for middleware in self.middlewares.iter() {
            // Destructure the request into its parts and body
            let (parts, body) = request.into_parts();

            let next = BoxCloneService::new(service_fn(|req: Request<Body>| {
                async move { Ok::<Response, Error>(Response::new(req.into_body())) }
            }));

            // Reconstruct the request with parts and body after middleware processing
            request = match middleware.handle(Request::from_parts(parts, body), next).await {
                Ok(response) => Request::new(response.into_body()), // Construct a new request from the response body
                Err(e) => return Err(e),
            };
        }

        Ok(request)
    }
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
        let chain = middleware_chain.clone();
        async move {
            // Execute custom middleware chain
            match chain.execute(req).await {
                Ok(req) => {
                    // Pass the successful request to the next handler in the chain
                    next.run(req).await
                }
                Err(err) => {
                    // Log the error
                    let error_message = format!("Middleware chain error: {}", err);
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