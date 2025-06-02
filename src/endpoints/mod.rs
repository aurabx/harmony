pub mod fhir;
pub mod jdx;
pub mod basic;
pub mod custom;

use axum::{
    Router,
};
use crate::config::{Config, EndpointKind};
use crate::middleware::build_middleware_stack;
use std::collections::HashMap;
use libloading::{Library, Symbol};

use crate::endpoints::fhir::FhirEndpointHandler;
use crate::endpoints::jdx::JdxEndpointHandler;
use crate::endpoints::basic::BasicEndpointHandler;
use crate::endpoints::custom::CustomEndpointFactory;


pub async fn build_router(config: &Config) -> Router {
    let mut app = Router::new();
    let mut factory = EndpointHandlerFactory::new();

    // Configure endpoints based on the config
    for (_name, endpoint) in &config.endpoints {
        let handler = match factory.create_handler(&endpoint.kind) {
            Ok(handler) => handler,
            Err(e) => {
                tracing::error!("Failed to create handler: {}", e);
                Box::new(BasicEndpointHandler)
            }
        };
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

pub struct EndpointHandlerFactory {
    loaded_libraries: HashMap<String, Library>,
}


impl EndpointHandlerFactory {
    pub fn new() -> Self {
        Self {
            loaded_libraries: HashMap::new(),
        }
    }

    pub fn create_handler(&mut self, kind: &EndpointKind) -> Result<Box<dyn EndpointHandler>, Box<dyn std::error::Error>> {
        match kind {
            EndpointKind::Basic => Ok(Box::new(BasicEndpointHandler)),
            EndpointKind::Fhir => Ok(Box::new(FhirEndpointHandler)),
            EndpointKind::Jdx => Ok(Box::new(JdxEndpointHandler)),
            EndpointKind::Custom { handler_path } => {
                // Load the dynamic library if not already loaded
                let lib = self.loaded_libraries.entry(handler_path.clone())
                    .or_insert_with(|| unsafe {
                        Library::new(handler_path).expect("Failed to load library")
                    });

                // Get the factory function
                let factory: Symbol<fn() -> Box<dyn CustomEndpointFactory>> =
                    unsafe { lib.get(b"create_endpoint_factory")? };

                let handler = factory().create_handler();
                Ok(handler)
            }
            _ => Ok(Box::new(BasicEndpointHandler)),
        }
    }
}




#[async_trait::async_trait]
pub trait EndpointHandler: Send + Sync {
    fn create_router(&self) -> Router;
}