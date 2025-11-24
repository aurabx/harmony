use crate::models::connection::ConnectionConfig;
use crate::models::services::services::{resolve_service, ServiceType};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Endpoint {
    pub service: String, // The service type, e.g., "http", "fhir", etc.
    #[serde(default)]
    pub options: Option<HashMap<String, serde_json::Value>>, // Service-specific options
    pub peer_ref: Option<String>,
    pub connection: Option<ConnectionConfig>,
    /// Authentication reference (DSL v1.9.0+): ID of global authentication definition
    pub authentication: Option<String>,
}

impl Endpoint {
    /// Resolves the service type using the centralized service resolver
    pub fn resolve_service(&self) -> Result<Box<dyn ServiceType<ReqBody = Value>>, String> {
        resolve_service(&self.service)
    }
}
