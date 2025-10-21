use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents an Envelope for passing data between endpoints, backends, and middleware.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestEnvelope<T> {
    /// Request details, such as method, headers, cookies, query params, cache status, and metadata.
    pub request_details: RequestDetails,
    /// Backend request details (for backward compatibility)
    pub backend_request_details: RequestDetails,
    /// Target details for the backend call (base_url, method, headers, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_details: Option<TargetDetails>,
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

/// Details about the backend target that the request will be sent to.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TargetDetails {
    /// Base URL of the target backend (e.g., "https://api.example.com")
    pub base_url: String,
    /// HTTP method (e.g., GET, POST) - can be overridden from RequestDetails
    pub method: String,
    /// Request URI or path (can be overridden from RequestDetails)
    pub uri: String,
    /// HTTP headers as key-value pairs (can be merged/overridden)
    pub headers: HashMap<String, String>,
    /// Cookies: name -> value (can be merged/overridden)
    pub cookies: HashMap<String, String>,
    /// Query parameters: name -> list of values (can be merged/overridden)
    pub query_params: HashMap<String, Vec<String>>,
    /// Additional target metadata
    pub metadata: HashMap<String, String>,
}

impl TargetDetails {
    /// Creates TargetDetails from RequestDetails with a base_url
    pub fn from_request_details(base_url: String, request_details: &RequestDetails) -> Self {
        Self {
            base_url,
            method: request_details.method.clone(),
            uri: request_details.uri.clone(),
            headers: request_details.headers.clone(),
            cookies: request_details.cookies.clone(),
            query_params: request_details.query_params.clone(),
            metadata: request_details.metadata.clone(),
        }
    }
    
    /// Constructs the full URL by combining base_url with uri and query_params
    pub fn full_url(&self) -> Result<String, crate::utils::Error> {
        let mut url = format!("{}{}", self.base_url, self.uri);
        
        // Add query parameters if any exist
        if !self.query_params.is_empty() {
            let params: Vec<String> = self.query_params
                .iter()
                .flat_map(|(key, values)| {
                    values.iter().map(move |value| {
                        format!("{}={}", 
                            urlencoding::encode(key), 
                            urlencoding::encode(value)
                        )
                    })
                })
                .collect();
            
            if !params.is_empty() {
                url.push('?');
                url.push_str(&params.join("&"));
            }
        }
        
        Ok(url)
    }
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
            target_details: None,
            original_data,
            normalized_data,
            normalized_snapshot: None,
        }
    }

    /// Creates a new RequestEnvelopeBuilder.
    /// 
    /// # Example
    /// 
    /// ```ignore
    /// let envelope = RequestEnvelope::builder()
    ///     .method("GET")
    ///     .uri("/test")
    ///     .original_data(b"test".to_vec())
    ///     .build()?;
    /// ```
    pub fn builder() -> RequestEnvelopeBuilder<T> {
        RequestEnvelopeBuilder::new()
    }
}

/// Builder for constructing RequestEnvelope instances with sensible defaults.
/// 
/// This builder simplifies creating RequestEnvelope instances by:
/// - Providing default values for optional fields (empty HashMaps, None)
/// - Automatically cloning request_details to backend_request_details
/// - Auto-normalizing original_data to JSON when T implements Serialize
/// 
/// # Examples
/// 
/// ## Minimal usage
/// ```ignore
/// let envelope = RequestEnvelopeBuilder::new()
///     .method("POST")
///     .uri("/api/resource")
///     .original_data(vec![1, 2, 3])
///     .build()?;
/// ```
/// 
/// ## From existing RequestDetails
/// ```ignore
/// let request_details = RequestDetails { /* ... */ };
/// let envelope = RequestEnvelopeBuilder::from_request_details(request_details)
///     .original_data(serde_json::json!({"key": "value"}))
///     .build()?;
/// ```
/// 
/// ## With metadata and headers
/// ```ignore
/// let envelope = RequestEnvelopeBuilder::new()
///     .method("GET")
///     .uri("/test")
///     .header("Content-Type", "application/json")
///     .metadata_entry("request_id", "123")
///     .original_data(vec![])
///     .build()?;
/// ```
#[derive(Debug, Clone)]
pub struct RequestEnvelopeBuilder<T> {
    method: Option<String>,
    uri: Option<String>,
    headers: HashMap<String, String>,
    cookies: HashMap<String, String>,
    query_params: HashMap<String, Vec<String>>,
    cache_status: Option<String>,
    metadata: HashMap<String, String>,
    backend_request_details: Option<RequestDetails>,
    target_details: Option<TargetDetails>,
    original_data: Option<T>,
    normalized_data: Option<serde_json::Value>,
    normalized_snapshot: Option<serde_json::Value>,
}

impl<T> RequestEnvelopeBuilder<T> {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self {
            method: None,
            uri: None,
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata: HashMap::new(),
            backend_request_details: None,
            target_details: None,
            original_data: None,
            normalized_data: None,
            normalized_snapshot: None,
        }
    }

    /// Creates a builder initialized with existing RequestDetails.
    pub fn from_request_details(details: RequestDetails) -> Self {
        Self {
            method: Some(details.method.clone()),
            uri: Some(details.uri.clone()),
            headers: details.headers.clone(),
            cookies: details.cookies.clone(),
            query_params: details.query_params.clone(),
            cache_status: details.cache_status.clone(),
            metadata: details.metadata.clone(),
            backend_request_details: None,
            target_details: None,
            original_data: None,
            normalized_data: None,
            normalized_snapshot: None,
        }
    }

    /// Creates a minimal envelope with only required fields.
    pub fn with_minimal(method: impl Into<String>, uri: impl Into<String>, original_data: T) -> Self {
        Self {
            method: Some(method.into()),
            uri: Some(uri.into()),
            original_data: Some(original_data),
            ..Self::new()
        }
    }

    /// Sets the HTTP method.
    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    /// Sets the request URI.
    pub fn uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
        self
    }

    /// Replaces all headers.
    pub fn headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    /// Adds a single header.
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Replaces all cookies.
    pub fn cookies(mut self, cookies: HashMap<String, String>) -> Self {
        self.cookies = cookies;
        self
    }

    /// Adds a single cookie.
    pub fn cookie(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.cookies.insert(key.into(), value.into());
        self
    }

    /// Replaces all query parameters.
    pub fn query_params(mut self, query_params: HashMap<String, Vec<String>>) -> Self {
        self.query_params = query_params;
        self
    }

    /// Adds a single query parameter.
    pub fn query_param(mut self, key: impl Into<String>, values: Vec<String>) -> Self {
        self.query_params.insert(key.into(), values);
        self
    }

    /// Sets the cache status.
    pub fn cache_status(mut self, cache_status: Option<String>) -> Self {
        self.cache_status = cache_status;
        self
    }

    /// Replaces all metadata.
    pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Adds a single metadata entry.
    pub fn metadata_entry(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Sets the backend request details explicitly.
    /// If not called, backend_request_details will be cloned from request_details.
    pub fn backend_request_details(mut self, details: RequestDetails) -> Self {
        self.backend_request_details = Some(details);
        self
    }

    /// Sets the target details.
    pub fn target_details(mut self, details: Option<TargetDetails>) -> Self {
        self.target_details = details;
        self
    }

    /// Sets the original data.
    pub fn original_data(mut self, data: T) -> Self {
        self.original_data = Some(data);
        self
    }

    /// Sets the normalized data explicitly.
    /// If not called and T implements Serialize, it will be auto-generated from original_data.
    pub fn normalized_data(mut self, data: Option<serde_json::Value>) -> Self {
        self.normalized_data = data;
        self
    }

    /// Sets the normalized snapshot.
    pub fn normalized_snapshot(mut self, snapshot: Option<serde_json::Value>) -> Self {
        self.normalized_snapshot = snapshot;
        self
    }
}

impl<T> RequestEnvelopeBuilder<T>
where
    T: Serialize,
{
    /// Builds the RequestEnvelope, validating required fields and applying defaults.
    /// 
    /// # Errors
    /// 
    /// Returns an error if:
    /// - method is not set
    /// - uri is not set  
    /// - original_data is not set
    pub fn build(self) -> Result<RequestEnvelope<T>, crate::utils::Error> {
        let method = self
            .method
            .ok_or_else(|| crate::utils::Error::from("method is required"))?;
        let uri = self
            .uri
            .ok_or_else(|| crate::utils::Error::from("uri is required"))?;
        let original_data = self
            .original_data
            .ok_or_else(|| crate::utils::Error::from("original_data is required"))?;

        // Build request_details
        let request_details = RequestDetails {
            method,
            uri,
            headers: self.headers,
            cookies: self.cookies,
            query_params: self.query_params,
            cache_status: self.cache_status,
            metadata: self.metadata,
        };

        // Auto-clone request_details to backend_request_details if not explicitly set
        let backend_request_details = self
            .backend_request_details
            .unwrap_or_else(|| request_details.clone());

        // Auto-normalize original_data if normalized_data not explicitly set
        let normalized_data = self
            .normalized_data
            .or_else(|| serde_json::to_value(&original_data).ok());

        Ok(RequestEnvelope {
            request_details,
            backend_request_details,
            target_details: self.target_details,
            original_data,
            normalized_data,
            normalized_snapshot: self.normalized_snapshot,
        })
    }
}

impl<T> Default for RequestEnvelopeBuilder<T> {
    fn default() -> Self {
        Self::new()
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
