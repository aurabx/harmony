use std::collections::HashMap;
use serde::Deserialize;
use serde_json::Value;
use crate::models::services::services::{ServiceType, resolve_service};

#[derive(Debug, Deserialize)]
pub struct Backend {
    pub service: String, // The service type, e.g., "http", "fhir", "dicom", etc.
    #[serde(default)]
    pub options: Option<HashMap<String, serde_json::Value>>, // Service-specific options
}

impl Backend {
    /// Resolves the service type using the centralized service resolver
    pub fn resolve_service(&self) -> Result<Box<dyn ServiceType<ReqBody=Value, ResBody=Value>>, String> {
        resolve_service(&self.service)
    }
}