use crate::config::config::ConfigError;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::router::route_config::RouteConfig;
use crate::utils::Error;

/// DICOM SCP (Service Class Provider) endpoint configuration
///
/// This service handles incoming DICOM DIMSE requests (C-FIND, C-MOVE, C-GET, C-STORE)
/// and routes them through the pipeline system.
#[derive(Debug, Deserialize)]
pub struct DicomScpEndpoint {
    /// Local Application Entity Title for the SCP listener
    pub local_aet: Option<String>,
    /// Bind address for the SCP listener (default: 0.0.0.0)
    pub bind_addr: Option<String>,
    /// Port for the SCP listener (default: 11112)
    pub port: Option<u16>,
    /// Enable C-ECHO operations
    pub enable_echo: Option<bool>,
    /// Enable C-FIND operations
    pub enable_find: Option<bool>,
    /// Enable C-MOVE operations
    pub enable_move: Option<bool>,
    /// Enable C-GET operations
    pub enable_get: Option<bool>,
    /// Storage directory for received DICOM files
    pub storage_dir: Option<String>,
}

impl DicomScpEndpoint {
    /// Get the local AET from options or struct, with default fallback
    fn get_local_aet(&self, options: &HashMap<String, Value>) -> String {
        options
            .get("local_aet")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.local_aet.clone())
            .unwrap_or_else(|| "HARMONY_SCP".to_string())
    }

    /// Get the port from options or struct, with default fallback
    fn get_port(&self, options: &HashMap<String, Value>) -> u16 {
        options
            .get("port")
            .and_then(|v| v.as_u64())
            .map(|p| p as u16)
            .or(self.port)
            .unwrap_or(11112) // Standard DICOM port
    }

    /// Get the bind address from options or struct, with default fallback
    fn get_bind_addr(&self, options: &HashMap<String, Value>) -> String {
        options
            .get("bind_addr")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.bind_addr.clone())
            .unwrap_or_else(|| "0.0.0.0".to_string())
    }
}

#[async_trait]
impl ServiceType for DicomScpEndpoint {
    fn required_protocol(&self) -> crate::models::protocol::Protocol {
        crate::models::protocol::Protocol::Dimse
    }

    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate local AET
        let local_aet = self.get_local_aet(options);
        if local_aet.trim().is_empty() || local_aet.len() > 16 {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom_scp".to_string(),
                reason: "Local AE title must be 1-16 characters".to_string(),
            });
        }

        // Validate port
        let port = self.get_port(options);
        if port == 0 {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom_scp".to_string(),
                reason: "Port must be non-zero".to_string(),
            });
        }

        // Validate bind address format (basic check)
        let bind_addr = self.get_bind_addr(options);
        if bind_addr.trim().is_empty() {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom_scp".to_string(),
                reason: "Bind address cannot be empty".to_string(),
            });
        }

        // Validate storage_dir if provided
        if let Some(dir) = options.get("storage_dir").and_then(|v| v.as_str()) {
            if dir.trim().is_empty() {
                return Err(ConfigError::InvalidEndpoint {
                    name: "dicom_scp".to_string(),
                    reason: "Storage directory cannot be empty".to_string(),
                });
            }
        }

        // Ensure at least one operation is enabled
        let enable_echo = options
            .get("enable_echo")
            .and_then(|v| v.as_bool())
            .or(self.enable_echo)
            .unwrap_or(true);
        let enable_find = options
            .get("enable_find")
            .and_then(|v| v.as_bool())
            .or(self.enable_find)
            .unwrap_or(false);
        let enable_move = options
            .get("enable_move")
            .and_then(|v| v.as_bool())
            .or(self.enable_move)
            .unwrap_or(false);
        let enable_get = options
            .get("enable_get")
            .and_then(|v| v.as_bool())
            .or(self.enable_get)
            .unwrap_or(false);

        if !enable_echo && !enable_find && !enable_move && !enable_get {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom_scp".to_string(),
                reason: "At least one DIMSE operation must be enabled (echo, find, move, or get)"
                    .to_string(),
            });
        }

        Ok(())
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // DICOM SCP does not register HTTP routes - it uses the DimseAdapter for protocol handling
        vec![]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        use crate::models::protocol::Protocol;
        use std::collections::HashMap as Map;

        if ctx.protocol != Protocol::Dimse {
            return Err(Error::from(
                "DicomScpEndpoint only supports Protocol::Dimse",
            ));
        }

        // Build RequestDetails from protocol context
        let metadata: Map<String, String> = ctx.meta.clone();
        let op = metadata
            .get("operation")
            .cloned()
            .unwrap_or_else(|| "DIMSE".into());
        let uri = format!("dimse://scp/{}", op.to_lowercase());

        // Parse normalized_data from payload if it's JSON
        let normalized: Option<serde_json::Value> = serde_json::from_slice(&ctx.payload).ok();

        RequestEnvelope::builder()
            .method(op)
            .uri(uri)
            .headers(Map::new())
            .cookies(Map::new())
            .query_params(Map::new())
            .cache_status(None)
            .metadata(metadata)
            .target_details(None)
            .original_data(ctx.payload)
            .normalized_data(normalized)
            .normalized_snapshot(None)
            .build()
    }
}

#[async_trait]
impl ServiceHandler<Value> for DicomScpEndpoint {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // SCP endpoint processes incoming DIMSE requests
        // The envelope has already been built by build_protocol_envelope
        // Here we can add additional validation or preprocessing if needed

        // Validate operation type
        let operation = envelope
            .request_details
            .metadata
            .get("operation")
            .ok_or_else(|| Error::from("Missing operation in DIMSE request"))?;

        // Ensure operation is supported
        let valid_ops = ["C-ECHO", "C-FIND", "C-MOVE", "C-GET", "C-STORE"];
        if !valid_ops.contains(&operation.as_str()) {
            return Err(Error::from(format!(
                "Unsupported DIMSE operation: {}",
                operation
            )));
        }

        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        _envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        // DicomScpEndpoint is not a backend - this should never be called
        Err(Error::from(
            "DicomScpEndpoint cannot be used as a backend (use dicom_scu instead)",
        ))
    }

    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Add protocol metadata to the response
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));
        envelope
            .response_details
            .metadata
            .insert("service".to_string(), "dicom_scp".to_string());
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Build HTTP response from ResponseEnvelope
        // This is primarily for debugging/monitoring, as DIMSE responses go through the protocol adapter
        let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        // Add headers from response_details
        for (k, v) in &envelope.response_details.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        // Use original_data if available, otherwise serialize normalized_data
        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else if let Some(normalized) = envelope.normalized_data {
            let body_bytes = serde_json::to_vec(&normalized)
                .map_err(|_| Error::from("Failed to serialize DICOM SCP response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct DICOM SCP HTTP response"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scp_validation_success() {
        let scp = DicomScpEndpoint {
            local_aet: Some("TEST_SCP".to_string()),
            bind_addr: Some("127.0.0.1".to_string()),
            port: Some(11112),
            enable_echo: Some(true),
            enable_find: None,
            enable_move: None,
            enable_get: None,
            storage_dir: None,
        };

        let options = HashMap::new();
        assert!(scp.validate(&options).is_ok());
    }

    #[test]
    fn test_scp_validation_invalid_aet() {
        let scp = DicomScpEndpoint {
            local_aet: Some("".to_string()),
            bind_addr: Some("127.0.0.1".to_string()),
            port: Some(11112),
            enable_echo: Some(true),
            enable_find: None,
            enable_move: None,
            enable_get: None,
            storage_dir: None,
        };

        let options = HashMap::new();
        assert!(scp.validate(&options).is_err());
    }

    #[test]
    fn test_scp_validation_no_operations_enabled() {
        let scp = DicomScpEndpoint {
            local_aet: Some("TEST_SCP".to_string()),
            bind_addr: Some("127.0.0.1".to_string()),
            port: Some(11112),
            enable_echo: Some(false),
            enable_find: Some(false),
            enable_move: Some(false),
            enable_get: Some(false),
            storage_dir: None,
        };

        let options = HashMap::new();
        assert!(scp.validate(&options).is_err());
    }

    #[test]
    fn test_get_local_aet_defaults() {
        let scp = DicomScpEndpoint {
            local_aet: None,
            bind_addr: None,
            port: None,
            enable_echo: None,
            enable_find: None,
            enable_move: None,
            enable_get: None,
            storage_dir: None,
        };

        let options = HashMap::new();
        assert_eq!(scp.get_local_aet(&options), "HARMONY_SCP");
    }

    #[test]
    fn test_get_port_defaults() {
        let scp = DicomScpEndpoint {
            local_aet: None,
            bind_addr: None,
            port: None,
            enable_echo: None,
            enable_find: None,
            enable_move: None,
            enable_get: None,
            storage_dir: None,
        };

        let options = HashMap::new();
        assert_eq!(scp.get_port(&options), 11112);
    }
}
