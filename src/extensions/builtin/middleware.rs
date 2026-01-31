//! Built-in middleware factories.
//!
//! Each built-in middleware type has a corresponding factory that creates
//! instances from configuration options.

use crate::config::config::Config;
use crate::extensions::MiddlewareFactory;
use crate::models::middleware::middleware::Middleware;
use serde_json::Value;
use std::collections::HashMap;

// ============================================================================
// JWT Auth Middleware
// ============================================================================

pub struct JwtAuthFactory;

impl MiddlewareFactory for JwtAuthFactory {
    fn type_name(&self) -> &str {
        "jwtauth"
    }

    fn aliases(&self) -> &[&str] {
        &["jwt_auth"]
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let config = crate::models::middleware::types::jwtauth::parse_config(options)?;
        Ok(Box::new(
            crate::models::middleware::types::jwtauth::JwtAuthMiddleware::new(config),
        ))
    }
}

// ============================================================================
// Basic Auth Middleware
// ============================================================================

pub struct BasicAuthFactory;

impl MiddlewareFactory for BasicAuthFactory {
    fn type_name(&self) -> &str {
        "basic_auth"
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let config = crate::models::middleware::types::auth::parse_config(options)?;
        Ok(Box::new(
            crate::models::middleware::types::auth::AuthSidecarMiddleware::new(config),
        ))
    }
}

// ============================================================================
// Connect Middleware
// ============================================================================

pub struct ConnectFactory;

impl MiddlewareFactory for ConnectFactory {
    fn type_name(&self) -> &str {
        "connect"
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let config = crate::models::middleware::types::connect::parse_config(options)?;
        Ok(Box::new(
            crate::models::middleware::types::connect::AuraboxConnectMiddleware::new(config),
        ))
    }
}

// ============================================================================
// Passthru Middleware
// ============================================================================

pub struct PassthruFactory;

impl MiddlewareFactory for PassthruFactory {
    fn type_name(&self) -> &str {
        "passthru"
    }

    fn create(
        &self,
        _options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        Ok(Box::new(
            crate::models::middleware::types::passthru::PassthruMiddleware::new(),
        ))
    }
}

// ============================================================================
// JSON Extractor Middleware
// ============================================================================

pub struct JsonExtractorFactory;

impl MiddlewareFactory for JsonExtractorFactory {
    fn type_name(&self) -> &str {
        "json_extractor"
    }

    fn aliases(&self) -> &[&str] {
        &["json"]
    }

    fn create(
        &self,
        _options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        Ok(Box::new(
            crate::models::middleware::types::json_extractor::JsonExtractorMiddleware::new(),
        ))
    }
}

// ============================================================================
// Jmix Builder Middleware
// ============================================================================

pub struct JmixBuilderFactory;

impl MiddlewareFactory for JmixBuilderFactory {
    fn type_name(&self) -> &str {
        "jmix_builder"
    }

    fn create(
        &self,
        _options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        Ok(Box::new(
            crate::models::middleware::types::jmix_builder::JmixBuilderMiddleware::new(),
        ))
    }
}

// ============================================================================
// DICOMweb Bridge Middleware
// ============================================================================

pub struct DicomwebBridgeFactory;

impl MiddlewareFactory for DicomwebBridgeFactory {
    fn type_name(&self) -> &str {
        "dicomweb_bridge"
    }

    fn aliases(&self) -> &[&str] {
        &["dicomweb"]
    }

    fn create(
        &self,
        _options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        Ok(Box::new(
            crate::models::middleware::types::dicomweb_to_dicom::DICOMwebToDICOMMiddleware::new(),
        ))
    }
}

// ============================================================================
// DICOM to DICOMweb Middleware
// ============================================================================

pub struct DicomToDicomwebFactory;

impl MiddlewareFactory for DicomToDicomwebFactory {
    fn type_name(&self) -> &str {
        "dicom_to_dicomweb"
    }

    fn create(
        &self,
        _options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        Ok(Box::new(
            crate::models::middleware::types::dicom_to_dicomweb::DicomToDicomwebMiddleware::new(),
        ))
    }
}

// ============================================================================
// DICOM Flatten Middleware
// ============================================================================

pub struct DicomFlattenFactory;

impl MiddlewareFactory for DicomFlattenFactory {
    fn type_name(&self) -> &str {
        "dicom_flatten"
    }

    fn aliases(&self) -> &[&str] {
        &["dicom_flatten_middleware"]
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let config = crate::models::middleware::types::dicom_flatten::parse_config(options)?;
        Ok(Box::new(
            crate::models::middleware::types::dicom_flatten::DicomFlattenMiddleware::new(config),
        ))
    }
}

// ============================================================================
// DICOM Unflatten Middleware
// ============================================================================

pub struct DicomUnflattenFactory;

impl MiddlewareFactory for DicomUnflattenFactory {
    fn type_name(&self) -> &str {
        "dicom_unflatten"
    }

    fn aliases(&self) -> &[&str] {
        &["dicom_unflatten_middleware"]
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let config = crate::models::middleware::types::dicom_unflatten::parse_config(options)?;
        Ok(Box::new(
            crate::models::middleware::types::dicom_unflatten::DicomUnflattenMiddleware::new(
                config,
            ),
        ))
    }
}

// ============================================================================
// Transform Middleware
// ============================================================================

pub struct TransformFactory;

impl MiddlewareFactory for TransformFactory {
    fn type_name(&self) -> &str {
        "transform"
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let transforms_path = config.and_then(|c| c.resolved_transforms_path.as_deref());
        let cfg =
            crate::models::middleware::types::transform::parse_config(options, transforms_path)?;
        Ok(Box::new(
            crate::models::middleware::types::transform::JoltTransformMiddleware::new(cfg)?,
        ))
    }
}

// ============================================================================
// Metadata Transform Middleware
// ============================================================================

pub struct MetadataTransformFactory;

impl MiddlewareFactory for MetadataTransformFactory {
    fn type_name(&self) -> &str {
        "metadata_transform"
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let transforms_path = config.and_then(|c| c.resolved_transforms_path.as_deref());
        let cfg = crate::models::middleware::types::metadata_transform::parse_config(
            options,
            transforms_path,
        )?;
        Ok(Box::new(
            crate::models::middleware::types::metadata_transform::MetadataTransformMiddleware::new(
                cfg,
            )?,
        ))
    }
}

// ============================================================================
// Path Filter Middleware
// ============================================================================

pub struct PathFilterFactory;

impl MiddlewareFactory for PathFilterFactory {
    fn type_name(&self) -> &str {
        "path_filter"
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let config = crate::models::middleware::types::path_filter::parse_config(options)?;
        Ok(Box::new(
            crate::models::middleware::types::path_filter::PathFilterMiddleware::new(config)?,
        ))
    }
}

// ============================================================================
// Policies Middleware
// ============================================================================

pub struct PoliciesFactory;

impl MiddlewareFactory for PoliciesFactory {
    fn type_name(&self) -> &str {
        "policies"
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let cfg = config.ok_or("Policies middleware requires Config context")?;
        let parsed_config = crate::models::middleware::types::policies::parse_config(
            options,
            &cfg.policies,
            &cfg.rules,
        )?;
        Ok(Box::new(
            crate::models::middleware::types::policies::PoliciesMiddleware::new(parsed_config)?,
        ))
    }
}

// ============================================================================
// Log Dump Middleware
// ============================================================================

pub struct LogDumpFactory;

impl MiddlewareFactory for LogDumpFactory {
    fn type_name(&self) -> &str {
        "log_dump"
    }

    fn aliases(&self) -> &[&str] {
        &["dump"]
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let config = crate::models::middleware::types::logger::parse_config(options)?;
        Ok(Box::new(
            crate::models::middleware::types::logger::LogDumpMiddleware::new(config),
        ))
    }
}

// ============================================================================
// Webhook Middleware
// ============================================================================

pub struct WebhookFactory;

impl MiddlewareFactory for WebhookFactory {
    fn type_name(&self) -> &str {
        "webhook"
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        _config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let config = crate::models::middleware::types::webhook::parse_config(options)?;
        Ok(Box::new(
            crate::models::middleware::types::webhook::WebhookMiddleware::new(config),
        ))
    }
}

// ============================================================================
// Mesh Auth Middleware
// ============================================================================

pub struct MeshAuthFactory;

impl MiddlewareFactory for MeshAuthFactory {
    fn type_name(&self) -> &str {
        "mesh_auth"
    }

    fn create(
        &self,
        options: &HashMap<String, Value>,
        config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String> {
        let mesh_config = crate::models::middleware::types::mesh_auth::parse_config_with_context(
            options, config,
        )?;
        Ok(Box::new(
            crate::models::middleware::types::mesh_auth::MeshAuthMiddleware::new(mesh_config),
        ))
    }
}
