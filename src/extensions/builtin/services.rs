//! Built-in service factories.
//!
//! Each built-in service type has a corresponding factory that creates
//! instances.

use crate::extensions::ServiceFactory;
use crate::models::services::services::ServiceType;
use serde_json::Value;

// ============================================================================
// HTTP Service
// ============================================================================

pub struct HttpServiceFactory;

impl ServiceFactory for HttpServiceFactory {
    fn service_name(&self) -> &str {
        "http"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::http::HttpEndpoint {})
    }
}

// ============================================================================
// HTTP/3 Service
// ============================================================================

pub struct Http3ServiceFactory;

impl ServiceFactory for Http3ServiceFactory {
    fn service_name(&self) -> &str {
        "http3"
    }

    fn aliases(&self) -> &[&str] {
        &["h3"]
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::http3::Http3Backend::default())
    }
}

// ============================================================================
// Echo Service
// ============================================================================

pub struct EchoServiceFactory;

impl ServiceFactory for EchoServiceFactory {
    fn service_name(&self) -> &str {
        "echo"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::echo::EchoEndpoint {})
    }
}

// ============================================================================
// FHIR Service
// ============================================================================

pub struct FhirServiceFactory;

impl ServiceFactory for FhirServiceFactory {
    fn service_name(&self) -> &str {
        "fhir"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::fhir::FhirEndpoint {})
    }
}

// ============================================================================
// DICOM SCU Service (Backend)
// ============================================================================

pub struct DicomScuServiceFactory;

impl ServiceFactory for DicomScuServiceFactory {
    fn service_name(&self) -> &str {
        "dicom_scu"
    }

    fn aliases(&self) -> &[&str] {
        &["dicom"] // Backward compatibility
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::dicom::DicomScuBackend {
            local_aet: None,
            aet: None,
            host: None,
            port: None,
            use_tls: None,
        })
    }
}

// ============================================================================
// DICOM SCP Service (Endpoint)
// ============================================================================

pub struct DicomScpServiceFactory;

impl ServiceFactory for DicomScpServiceFactory {
    fn service_name(&self) -> &str {
        "dicom_scp"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(
            crate::models::services::types::dicom_scp::DicomScpEndpoint {
                local_aet: None,
                bind_addr: None,
                port: None,
                enable_echo: None,
                enable_find: None,
                enable_move: None,
                enable_get: None,
                enable_store: None,
                storage_dir: None,
            },
        )
    }
}

// ============================================================================
// DICOMweb Service
// ============================================================================

pub struct DicomwebServiceFactory;

impl ServiceFactory for DicomwebServiceFactory {
    fn service_name(&self) -> &str {
        "dicomweb"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::dicomweb::DicomwebEndpoint {})
    }
}

// ============================================================================
// Mock DICOM Service
// ============================================================================

pub struct MockDicomServiceFactory;

impl ServiceFactory for MockDicomServiceFactory {
    fn service_name(&self) -> &str {
        "mock_dicom"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::mock_dicom::MockDicomEndpoint {})
    }
}

// ============================================================================
// Jmix Service (Endpoint)
// ============================================================================

pub struct JmixServiceFactory;

impl ServiceFactory for JmixServiceFactory {
    fn service_name(&self) -> &str {
        "jmix"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::jmix::JmixEndpoint {})
    }
}

// ============================================================================
// Jmix Backend Service
// ============================================================================

pub struct JmixBackendServiceFactory;

impl ServiceFactory for JmixBackendServiceFactory {
    fn service_name(&self) -> &str {
        "jmix_backend"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::jmix_backend::JmixBackend {})
    }
}

// ============================================================================
// Storage Service
// ============================================================================

pub struct StorageServiceFactory;

impl ServiceFactory for StorageServiceFactory {
    fn service_name(&self) -> &str {
        "storage"
    }

    fn aliases(&self) -> &[&str] {
        &["disk"]
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::models::services::types::storage::StorageBackend::default())
    }
}

// ============================================================================
// Management Service
// ============================================================================

pub struct ManagementServiceFactory;

impl ServiceFactory for ManagementServiceFactory {
    fn service_name(&self) -> &str {
        "management"
    }

    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
        Box::new(crate::management::ManagementEndpoint {})
    }
}
