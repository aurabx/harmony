use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Represents an Envelope for passing data between endpoints, backends, and middleware.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestEnvelope<T> {
    /// Request details, such as method, headers, and metadata.
    pub request_details: RequestDetails,
    /// Original data received from the source (not serialized).
    #[serde(skip)]
    pub original_data: T,
    /// A normalized JSON representation of the original data.
    pub normalized_data: Option<serde_json::Value>,
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
    /// Additional metadata, if necessary.
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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
    pub response_details: ResponseDetails,
    #[serde(skip)]
    #[allow(dead_code)]
    pub original_data: T,
    pub normalized_data: Option<serde_json::Value>,
}

impl<T> RequestEnvelope<T>
where
    T: Serialize,
{
    /// Creates a new Envelope instance with request details and normalized data.
    pub fn new(request_details: RequestDetails, original_data: T) -> Self {
        let normalized_data = serde_json::to_value(&original_data).ok();
        RequestEnvelope {
            request_details,
            original_data,
            normalized_data,
        }
    }
}
