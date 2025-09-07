use std::sync::Arc;
use axum::body::Body;
use axum::response::Response;
use http::Request;
use tower::service_fn;
use tower::util::BoxCloneService;
use crate::models::middleware::auth::AuthSidecarMiddleware;
use crate::models::middleware::config::MiddlewareConfig;
use crate::models::middleware::connect::AuraboxConnectMiddleware;
use crate::models::middleware::jwtauth::JwtAuthMiddleware;
use crate::models::middleware::Middleware;
use crate::models::middleware::types::Error;

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

    pub async fn left(&self, mut request: Request<Body>) -> Result<Request<Body>, Error> {
        for middleware in self.middlewares.iter() {
            // Destructure the request into its parts and body
            let (parts, body) = request.into_parts();

            let next = BoxCloneService::new(service_fn(|req: Request<Body>| {
                async move { Ok::<Response, Error>(Response::new(req.into_body())) }
            }));

            // Reconstruct the request with parts and body after middleware processing
            request = match middleware.left(Request::from_parts(parts, body), next).await {
                Ok(response) => Request::new(response.into_body()), // Construct a new request from the response body
                Err(e) => return Err(e),
            };
        }

        Ok(request)
    }

    pub async fn right(&self, mut request: Request<Body>) -> Result<Request<Body>, Error> {
        for middleware in self.middlewares.iter() {
            // Destructure the request into its parts and body
            let (parts, body) = request.into_parts();

            let next = BoxCloneService::new(service_fn(|req: Request<Body>| {
                async move { Ok::<Response, Error>(Response::new(req.into_body())) }
            }));

            // Reconstruct the request with parts and body after middleware processing
            request = match middleware.right(Request::from_parts(parts, body), next).await {
                Ok(response) => Request::new(response.into_body()), // Construct a new request from the response body
                Err(e) => return Err(e),
            };
        }

        Ok(request)
    }
}