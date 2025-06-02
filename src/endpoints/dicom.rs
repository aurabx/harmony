
use axum::Router;
use super::EndpointHandler;

#[allow(dead_code)]
pub struct DicomEndpointHandler {
    pub aet: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
}

impl DicomEndpointHandler {
    pub fn new(aet: Option<String>, host: Option<String>, port: Option<u16>) -> Self {
        Self { aet, host, port }
    }
}

impl EndpointHandler for DicomEndpointHandler {
    fn create_router(&self) -> Router {
        // TODO: Implement DICOM-specific routing logic here
        Router::new()
    }
}