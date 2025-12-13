use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

/// Configuration for DICOM unflatten middleware
#[derive(Debug, Deserialize, Clone)]
pub struct DicomUnflattenConfig {}

/// Parses middleware configuration from options
pub fn parse_config(_options: &std::collections::HashMap<String, Value>) -> Result<DicomUnflattenConfig, String> {
    Ok(DicomUnflattenConfig {})
}

/// DICOM JSON unflattening middleware
/// Converts flat key-value pairs back to standard DICOM JSON format (with vr/Value)
pub struct DicomUnflattenMiddleware {}

impl DicomUnflattenMiddleware {
    pub fn new(_config: DicomUnflattenConfig) -> Self {
        Self {}
    }
}

#[async_trait]
impl Middleware for DicomUnflattenMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<Value>, Error> {
        if let Some(ref data) = envelope.normalized_data {
            // Store snapshot before transformation if not already present
            if envelope.normalized_snapshot.is_none() {
                envelope.normalized_snapshot = Some(data.clone());
            }

            match super::dicom_flatten::unflatten_dicom_json(data) {
                Ok(unflattened) => {
                    envelope.normalized_data = Some(unflattened);
                    tracing::debug!("Applied DICOM unflatten on request");
                }
                Err(e) => {
                    tracing::error!("DICOM unflatten failed: {}", e);
                    return Err(Error::from(format!("DICOM unflatten failed: {}", e)));
                }
            }
        }

        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<Value>,
    ) -> Result<ResponseEnvelope<Value>, Error> {
        // Unflatten is typically used on requests, not responses
        Ok(envelope)
    }
}
