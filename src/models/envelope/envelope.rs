use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents an Envelope for passing data between endpoints, backends, and middleware.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestEnvelope<T> {
    /// Request details, such as method, headers, cookies, query params, cache status, and metadata.
    pub request_details: RequestDetails,
    /// Backend request details
    pub backend_request_details: RequestDetails,
    /// Original data received from the source (not serialized).
    #[serde(skip)]
    pub original_data: T,
    /// A normalized JSON representation of the original data.
    pub normalized_data: Option<serde_json::Value>,
    /// Snapshot of normalized_data before any transform middleware is applied.
    /// Only populated when transform middleware is used to preserve original state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_snapshot: Option<serde_json::Value>,
}

/// Details about the request being processed.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestDetails {
    /// HTTP method (e.g., GET, POST).
    pub method: String,
    /// Request URI or path.
    pub uri: String,
    /// HTTP headers as key-value pairs.
    pub headers: HashMap<String, String>,
    /// Cookies parsed from the Cookie header(s): name -> value.
    pub cookies: HashMap<String, String>,
    /// Query parameters parsed from the request URI: name -> list of values.
    pub query_params: HashMap<String, Vec<String>>,
    /// Cache status derived from common cache headers (e.g., Cache-Status, X-Cache, CF-Cache-Status).
    pub cache_status: Option<String>,
    /// Additional metadata, if necessary.
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
pub struct ResponseDetails {
    /// HTTP status code
    pub status: u16,
    /// Response headers as key-value pairs
    pub headers: HashMap<String, String>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseEnvelope<T> {
    /// Original request details for context
    pub request_details: RequestDetails,
    /// Response details including status, headers, metadata
    pub response_details: ResponseDetails,
    #[serde(skip)]
    pub original_data: T,
    pub normalized_data: Option<serde_json::Value>,
    /// Snapshot of normalized_data before any transform middleware is applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_snapshot: Option<serde_json::Value>,
}

impl<T> RequestEnvelope<T>
where
    T: Serialize,
{
    /// Creates a new Envelope instance with request details and normalized data.
    pub fn new(request_details: RequestDetails, original_data: T) -> Self {
        let normalized_data = serde_json::to_value(&original_data).ok();

        let backend_request_details = request_details.clone();

        RequestEnvelope {
            request_details,
            backend_request_details,
            original_data,
            normalized_data,
            normalized_snapshot: None,
        }
    }
}

impl ResponseEnvelope<Vec<u8>> {
    /// Creates a new ResponseEnvelope from backend response components
    pub fn from_backend(
        request_details: RequestDetails,
        status: u16,
        headers: HashMap<String, String>,
        body: Vec<u8>,
        metadata: Option<HashMap<String, String>>,
    ) -> Self {
        let response_details = ResponseDetails {
            status,
            headers,
            metadata: metadata.unwrap_or_default(),
        };

        ResponseEnvelope {
            request_details,
            response_details,
            original_data: body,
            normalized_data: None,
            normalized_snapshot: None,
        }
    }

    /// Converts to JSON-aware envelope by parsing body if content-type indicates JSON
    pub fn to_json(self) -> Result<ResponseEnvelope<serde_json::Value>, crate::utils::Error> {
        // If normalized_data already exists AND body is empty, preserve normalized_data
        // This handles cases where middleware sets metadata that needs to be preserved
        // but the actual response body should come from original_data
        if let Some(existing_normalized) = self.normalized_data.clone() {
            // Only preserve as original_data if body is empty
            if self.original_data.is_empty() {
                return Ok(ResponseEnvelope {
                    request_details: self.request_details,
                    response_details: self.response_details,
                    original_data: existing_normalized.clone(),
                    normalized_data: Some(existing_normalized),
                    normalized_snapshot: self.normalized_snapshot,
                });
            }
            // Body is not empty - parse it and preserve normalized_data separately
            // (normalized_data will be preserved below)
        }

        // Check if content-type indicates JSON
        let is_json = self
            .response_details
            .headers
            .get("content-type")
            .or_else(|| self.response_details.headers.get("Content-Type"))
            .map(|ct| {
                ct.contains("application/json")
                    || ct.contains("application/fhir+json")
                    || ct.contains("application/dicom+json")
                    || ct.contains("+json")
            })
            .unwrap_or(false);

        let (parsed_normalized, original_json) = if is_json && !self.original_data.is_empty() {
            match serde_json::from_slice::<serde_json::Value>(&self.original_data) {
                Ok(json) => (Some(json.clone()), json),
                Err(_) => {
                    // Failed to parse as JSON, keep as null but preserve bytes
                    (None, serde_json::Value::Null)
                }
            }
        } else {
            // Not JSON or empty body
            (None, serde_json::Value::Null)
        };

        // Preserve existing normalized_data if it exists, otherwise use parsed data
        let final_normalized = if self.normalized_data.is_some() {
            self.normalized_data
        } else {
            parsed_normalized
        };

        Ok(ResponseEnvelope {
            request_details: self.request_details,
            response_details: self.response_details,
            original_data: original_json,
            normalized_data: final_normalized,
            normalized_snapshot: self.normalized_snapshot,
        })
    }
}

impl ResponseEnvelope<serde_json::Value> {
    /// Converts back to byte-level envelope, serializing JSON if needed
    pub fn to_bytes(mut self) -> Result<ResponseEnvelope<Vec<u8>>, crate::utils::Error> {
        // Prefer normalized_data for body (may have been modified by middleware)
        // Fall back to original_data if normalized_data is None
        let body_bytes = if let Some(ref normalized) = self.normalized_data {
            let bytes = serde_json::to_vec(normalized).map_err(|e| {
                crate::utils::Error::from(format!("Failed to serialize normalized_data: {}", e))
            })?;

            // Ensure content-type is set to application/json if not already set
            if !self.response_details.headers.contains_key("content-type")
                && !self.response_details.headers.contains_key("Content-Type")
            {
                self.response_details
                    .headers
                    .insert("content-type".to_string(), "application/json".to_string());
            }

            bytes
        } else if self.original_data != serde_json::Value::Null {
            // Use original_data if normalized_data is None
            serde_json::to_vec(&self.original_data).map_err(|e| {
                crate::utils::Error::from(format!("Failed to serialize original_data: {}", e))
            })?
        } else {
            // Empty body
            Vec::new()
        };

        Ok(ResponseEnvelope {
            request_details: self.request_details,
            response_details: self.response_details,
            original_data: body_bytes,
            normalized_data: self.normalized_data,
            normalized_snapshot: self.normalized_snapshot,
        })
    }
}
