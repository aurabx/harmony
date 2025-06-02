pub mod fhir;
pub mod jdx;
pub mod basic;
pub mod custom;
pub mod config;
mod dicom;

use axum::{
    Router,
};
use std::collections::HashMap;
use libloading::{Library, Symbol};

use crate::endpoints::fhir::FhirEndpointHandler;
use crate::endpoints::jdx::JdxEndpointHandler;
use crate::endpoints::basic::BasicEndpointHandler;
use crate::endpoints::dicom::DicomEndpointHandler;
use crate::endpoints::custom::CustomEndpointFactory;
use crate::endpoints::config::EndpointKind;


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
            EndpointKind::Basic { path_prefix: _ } => Ok(Box::new(BasicEndpointHandler)),
            EndpointKind::Fhir { path_prefix: _ } => Ok(Box::new(FhirEndpointHandler)),
            EndpointKind::Jdx { path_prefix: _ } => Ok(Box::new(JdxEndpointHandler)),
            EndpointKind::Dicom { aet, host, port } => {
                Ok(Box::new(DicomEndpointHandler::new(
                    aet.clone(),
                    host.clone(),
                    *port,
                )))
            },
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
        }
    }
}

#[async_trait::async_trait]
pub trait EndpointHandler: Send + Sync {
    fn create_router(&self) -> Router;
}