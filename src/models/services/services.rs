use crate::config::config::Config;
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::response::Response;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct ServiceConfig {
    pub module: String, // Path to the module or metadata
}

// Use OnceCell for one-time initialization of SERVICE_REGISTRY
pub static SERVICE_REGISTRY: OnceCell<HashMap<String, String>> = OnceCell::new();

pub fn initialise_service_registry(config: &Config) {
    // Populate the registry using the service types from the provided config
    let registry = config
        .services
        .iter()
        .map(|(key, value)| (key.clone(), value.module.clone()))
        .collect();

    // Set the SERVICE_REGISTRY value; this will panic if called more than once
    SERVICE_REGISTRY
        .set(registry)
        .expect("SERVICE_REGISTRY can only be initialized once");
}

/// Resolves a service type from the registry and returns a boxed ServiceType
/// This function can be used by both Endpoints and Backends
pub fn resolve_service(
    service_type: &str,
) -> Result<Box<dyn ServiceType<ReqBody = Value>>, String> {
    // Check the registry first
    if let Some(registry) = SERVICE_REGISTRY.get() {
        if let Some(module) = registry.get(service_type) {
            match module.as_str() {
                "" => {
                    // Default built-in modules
                    create_builtin_service(service_type)
                }
                module_path => {
                    // Custom module loading would go here
                    Err(format!(
                        "Service type '{}' references module '{}' but dynamic loading is not implemented yet",
                        service_type, module_path
                    ))
                }
            }
        } else {
            Err(format!("Unknown service type: {}", service_type))
        }
    } else {
        // Fallback to hardcoded types if registry isn't initialized
        create_builtin_service(service_type)
    }
}

/// Creates built-in service instances
fn create_builtin_service(
    service_type: &str,
) -> Result<Box<dyn ServiceType<ReqBody = Value>>, String> {
    match service_type.to_lowercase().as_str() {
        "http" => Ok(Box::new(
            crate::models::services::types::http::HttpEndpoint {},
        )),
        "jmix" => Ok(Box::new(
            crate::models::services::types::jmix::JmixEndpoint {},
        )),
        "fhir" => Ok(Box::new(
            crate::models::services::types::fhir::FhirEndpoint {},
        )),
        "dicom" => Ok(Box::new(
            crate::models::services::types::dicom::DicomEndpoint {
                local_aet: None,
                aet: None,
                host: None,
                port: None,
                use_tls: None,
            },
        )),
        "dicomweb" => Ok(Box::new(
            crate::models::services::types::dicomweb::DicomwebEndpoint {},
        )),
        "echo" => Ok(Box::new(
            crate::models::services::types::echo::EchoEndpoint {},
        )),
        _ => Err(format!(
            "Unsupported built-in service type: {}",
            service_type
        )),
    }
}

#[async_trait]
pub trait ServiceType: ServiceHandler<Value> {
    /// Validate the service configuration
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError>;

    /// Returns configured routes
    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig>;

    /// Protocol-agnostic envelope builder. Default: unsupported.
    async fn build_protocol_envelope(
        &self,
        _ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "build_protocol_envelope is not supported by this service",
        ))
    }
}

#[async_trait]
pub trait ServiceHandler<T>: Send + Sync
where
    T: Send,
{
    type ReqBody: Send;
    // Response body is determined by the Service; use axum Body

    /// Handles incoming requests, producing an Envelope
    async fn transform_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error>;

    /// Handles the response stage, converting Envelope back into an HTTP response
    async fn transform_response(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Response, Error>;
}
