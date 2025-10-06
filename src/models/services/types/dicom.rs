use std::collections::HashMap;
use async_trait::async_trait;
use axum::{response::Response, body::Body};
use serde_json::Value;
use serde::Deserialize;
use crate::config::config::ConfigError;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::services::services::{ServiceType, ServiceHandler};

use crate::utils::Error;
use crate::router::route_config::RouteConfig;
use dimse::{DimseConfig, RemoteNode, DimseScu};
use dimse::types::{FindQuery, QueryLevel};

#[derive(Debug, Deserialize)]
pub struct DicomEndpoint {
    pub local_aet: Option<String>,
    pub aet: Option<String>,  // For backward compatibility (remote AET)
    pub host: Option<String>,
    pub port: Option<u16>,
    pub use_tls: Option<bool>,
}

impl DicomEndpoint {
    /// Check if this is being used as a backend (SCU) vs endpoint (SCP)
    fn is_backend_usage(&self, options: &HashMap<String, Value>) -> bool {
        // If host/aet are provided, it's for backend usage (connecting to remote)
        // Note: 'port' alone can be used for SCP listener and should NOT imply backend usage
        options.contains_key("host") || options.contains_key("aet")
    }

    /// Get the local AET from options or struct
    fn get_local_aet(&self, options: &HashMap<String, Value>) -> Option<String> {
        options.get("local_aet")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.local_aet.clone())
            .or_else(|| Some("HARMONY_DICOM".to_string()))
    }

    /// Create a remote node from configuration
    fn create_remote_node(&self, options: &HashMap<String, Value>) -> Result<RemoteNode, ConfigError> {
        let aet = options.get("aet")
            .and_then(|v| v.as_str())
            .or(self.aet.as_deref())
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Missing remote 'aet' (Application Entity Title)".to_string(),
            })?.to_string();

        let host = options.get("host")
            .and_then(|v| v.as_str())
            .or(self.host.as_deref())
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Missing 'host' (DICOM server address)".to_string(),
            })?.to_string();

        let port = options.get("port")
            .and_then(|v| v.as_u64())
            .or(self.port.map(|p| p as u64))
            .ok_or_else(|| ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Missing 'port'".to_string(),
            })?;

        // DICOM servers commonly use privileged ports like 104, so allow 1-65535 for remote nodes
        if !(1..=65535).contains(&port) {
            return Err(ConfigError::InvalidEndpoint {
                name: "dicom".to_string(),
                reason: "Invalid 'port' (Allowed range: 1-65535)".to_string(),
            });
        }

        let mut node = RemoteNode::new(aet, host, port as u16);
        
        if options.get("use_tls")
            .and_then(|v| v.as_bool())
            .or(self.use_tls)
            .unwrap_or(false) {
            node = node.with_tls();
        }

        Ok(node)
    }
}

impl ServiceType for DicomEndpoint {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        if self.is_backend_usage(options) {
            // Backend usage - validate remote connection parameters
            self.create_remote_node(options)?;
        } else {
            // Endpoint usage - validate local AET only for SCP listener
            let local_aet = self.get_local_aet(options)
                .ok_or_else(|| ConfigError::InvalidEndpoint {
                    name: "dicom".to_string(),
                    reason: "Missing 'local_aet' for DICOM endpoint (SCP)".to_string(),
                })?;
            
            if local_aet.trim().is_empty() || local_aet.len() > 16 {
                return Err(ConfigError::InvalidEndpoint {
                    name: "dicom".to_string(),
                    reason: "Local AE title must be 1-16 characters".to_string(),
                });
            }

            // Optional: validate port if provided
            if let Some(port_val) = options.get("port").and_then(|v| v.as_u64()) {
                if port_val == 0 || port_val > 65535 {
                    return Err(ConfigError::InvalidEndpoint {
                        name: "dicom".to_string(),
                        reason: "Invalid 'port' (Allowed range: 1-65535)".to_string(),
                    });
                }
            }
        }
        
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        if self.is_backend_usage(options) {
            // Backend usage - no HTTP routes needed (DIMSE protocol only)
            vec![]
        } else {
            // Endpoint usage - no HTTP routes; SCP listener is started by the router/dispatcher with pipeline context
            vec![]
        }
    }
}

#[async_trait]
impl ServiceHandler<Value> for DicomEndpoint {
    type ReqBody = Value;

    async fn transform_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        if self.is_backend_usage(options) {
            // Backend usage - prepare for DIMSE SCU operations
            self.handle_backend_request(&mut envelope, options).await
        } else {
            // Misconfiguration: DICOM cannot act as HTTP endpoint
            Err(Error::from("DICOM service cannot be used as an endpoint; configure an HTTP endpoint and a DICOM backend instead"))
        }
    }

    async fn transform_response(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        let nd = envelope.normalized_data.unwrap_or(serde_json::Value::Null);
        let response_meta = nd.get("response");

        let status = response_meta
            .and_then(|m| m.get("status"))
            .and_then(|s| s.as_u64())
            .and_then(|code| http::StatusCode::from_u16(code as u16).ok())
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);
        let mut has_content_type = false;
        if let Some(hdrs) = response_meta.and_then(|m| m.get("headers")).and_then(|h| h.as_object()) {
            for (k, v) in hdrs.iter() {
                if let Some(val_str) = v.as_str() {
                    if k.eq_ignore_ascii_case("content-type") { has_content_type = true; }
                    builder = builder.header(k.as_str(), val_str);
                }
            }
        }

        if let Some(body_str) = response_meta.and_then(|m| m.get("body")).and_then(|b| b.as_str()) {
            return builder
                .body(Body::from(body_str.to_string()))
                .map_err(|_| Error::from("Failed to construct DICOM HTTP response"));
        }

        let body_str = serde_json::to_string(&nd).map_err(|_| Error::from("Failed to serialize DICOM response payload into JSON"))?;
        if !has_content_type {
            builder = builder.header("content-type", "application/json");
        }
        builder
            .body(Body::from(body_str))
            .map_err(|_| Error::from("Failed to construct DICOM HTTP response"))
    }
}

impl DicomEndpoint {
    /// Handle backend (SCU) request processing
    async fn handle_backend_request(
        &self,
        envelope: &mut RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Create remote node configuration
        let remote_node = self.create_remote_node(options)
            .map_err(|e| Error::from(format!("Failed to create remote node: {:?}", e)))?;

        // Create DIMSE SCU configuration
        let local_aet = self.get_local_aet(options)
            .unwrap_or_else(|| "HARMONY_SCU".to_string());
        
        let dimse_config = DimseConfig {
            local_aet,
            ..Default::default()
        };

        // Create SCU client
        let scu = DimseScu::new(dimse_config);

        // Extract path to determine operation type
        let path = envelope.request_details.metadata.get("path")
            .cloned()
            .unwrap_or_default();

        let result = match path.as_str() {
            "echo" | "/echo" => {
                // Perform C-ECHO
                match scu.echo(&remote_node).await {
                    Ok(success) => serde_json::json!({
                        "operation": "echo",
                        "success": success,
                        "remote_aet": remote_node.ae_title,
                        "host": remote_node.host,
                        "port": remote_node.port
                    }),
                    Err(e) => serde_json::json!({
                        "operation": "echo",
                        "success": false,
                        "error": e.to_string()
                    })
                }
            },
            "find" | "/find" => {
                // Parse request body for query parameters
                let query_params: HashMap<String, String> = serde_json::from_slice(&envelope.original_data)
                    .unwrap_or_default();
                
                let query_level = query_params.get("query_level")
                    .and_then(|level| level.parse::<QueryLevel>().ok())
                    .unwrap_or(QueryLevel::Patient);
                
                let mut query = FindQuery::patient(query_params.get("patient_id").cloned());
                query.query_level = query_level;
                
                // Add other query parameters
                for (key, value) in query_params {
                    if !key.starts_with("query_") {
                        query = query.with_parameter(key, value);
                    }
                }

                // Perform C-FIND (for now, just return query info)
                match scu.find(&remote_node, query).await {
                    Ok(_stream) => serde_json::json!({
                        "operation": "find",
                        "success": true,
                        "message": "C-FIND initiated (streaming not yet implemented)"
                    }),
                    Err(e) => serde_json::json!({
                        "operation": "find",
                        "success": false,
                        "error": e.to_string()
                    })
                }
            },
            _ => serde_json::json!({
                "operation": "unknown",
                "success": false,
                "error": format!("Unknown DIMSE operation: {}", path)
            })
        };

        envelope.normalized_data = Some(result);
        Ok(envelope.clone())
    }
}

