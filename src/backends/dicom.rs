use crate::backends::config::{Backend, BackendType};
use crate::config::ConfigError;
use crate::endpoints::config::{Endpoint, EndpointKind};

pub(crate) fn validate_dicom_backend(backend: &Backend) -> Result<(), ConfigError> {
    match &backend.type_ {
        BackendType::Dicom { port, .. } => {
            if *port == 0 {
                return Err(ConfigError::InvalidBackend {
                    name: "unknown".to_string(), // You might want to pass the backend name here
                    reason: "DICOM backend requires a non-zero port".to_string(),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_dicom_endpoint(endpoint: &Endpoint) -> Result<(), ConfigError> {
    if let EndpointKind::Dicom { port, .. } = &endpoint.kind {
        if port.is_none() {
            return Err(ConfigError::InvalidEndpoint {
                name: endpoint.path_prefix.clone(),
                reason: "DICOM endpoint requires a port".to_string(),
            });
        }
    }
    Ok(())
}
