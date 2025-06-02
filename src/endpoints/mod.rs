mod fhir;
mod jdx;
mod basic;

use axum::{
    Router,
};

use crate::config::{Config, EndpointKind};
use crate::middleware::build_middleware_stack;

use crate::endpoints::fhir::FhirEndpointHandler;
use crate::endpoints::jdx::JdxEndpointHandler;
use crate::endpoints::basic::BasicEndpointHandler;

pub async fn build_router(config: &Config) -> Router {
    let mut app = Router::new();

    // Configure endpoints based on the config
    for (_name, endpoint) in &config.endpoints {
        let handler = EndpointHandlerFactory::create_handler(&endpoint.kind);
        let mut route = handler.create_router();

        // Add configured middleware if any
        if let Some(middleware_list) = &endpoint.middleware {
            route = build_middleware_stack(route, middleware_list, &config.middleware);
        }

        // Mount the route at the configured path prefix
        app = app.nest(&endpoint.path_prefix, route);
    }

    app
}

pub struct EndpointHandlerFactory;

impl EndpointHandlerFactory {
    pub fn create_handler(kind: &EndpointKind) -> Box<dyn EndpointHandler> {
        match kind {
            EndpointKind::Basic => Box::new(BasicEndpointHandler),
            EndpointKind::Fhir => Box::new(FhirEndpointHandler),
            EndpointKind::Jdx => Box::new(JdxEndpointHandler),
            _ => Box::new(BasicEndpointHandler), // Default or handle other cases
        }
    }
}

#[async_trait::async_trait]
pub trait EndpointHandler: Send + Sync {
    fn create_router(&self) -> Router;
}