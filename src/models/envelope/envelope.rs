use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::command::{Command, Operation};

/// Parse status for content parsing operations
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParseStatus {
    /// Content was successfully parsed
    Success,
    /// Parsing failed with an error
    Failed,
    /// Parsing was not attempted
    NotAttempted,
    /// Content type is not supported for parsing
    Unsupported,
}

/// Metadata about content type and parsing status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentMetadata {
    /// The Content-Type header value
    pub content_type: String,
    /// Character encoding (e.g., "utf-8")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    /// Format identifier ("json", "xml", "csv", "form", "multipart", "binary")
    pub format: String,
    /// Result of parsing operation
    pub parse_status: ParseStatus,
    /// Size of original data in bytes
    pub original_size: usize,
    /// SHA256 checksum for binary content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// A DICOM Part 10 file this process wrote for one request. The DIMSE
/// adapter makes one for each C-STORE instance it receives.
///
/// Only code in this crate can create one, and serde never reads or writes
/// it. So a client cannot point a backend at a file on this host. Backends
/// read the instance from here instead of from a path in the request body.
#[derive(Debug, Clone)]
pub struct LocalInstanceFile {
    path: PathBuf,
}

impl LocalInstanceFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

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
    /// The intended operation for the backend (set by endpoint or transform).
    /// Backends interpret this according to their protocol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
    /// Original data received from the source (not serialized).
    #[serde(skip)]
    pub original_data: T,
    /// A normalized JSON representation of the original data.
    pub normalized_data: Option<serde_json::Value>,
    /// Snapshot of normalized_data before any transform middleware is applied.
    /// Only populated when transform middleware is used to preserve original state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_snapshot: Option<serde_json::Value>,
    /// The local copy of the request's DICOM instance, if the endpoint made
    /// one. The executor hides it from middleware.
    #[serde(skip)]
    pub local_instance: Option<LocalInstanceFile>,
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

impl Default for TargetDetails {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            method: String::new(),
            uri: String::new(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            metadata: HashMap::new(),
        }
    }
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

    /// Reject a `uri` whose path holds a `.` or `..` segment.
    ///
    /// The HTTP adapters reject these on the client's path, but middleware can
    /// write `uri` later, for example a JOLT spec that copies a client value.
    /// The HTTP client would resolve the segment and leave the backend's base
    /// path with the target credential. Every send path calls this, including
    /// those that do not use [`Self::full_url`]. The query and fragment are not
    /// checked, because the HTTP client does not resolve them.
    pub fn reject_dot_segment_uri(&self) -> Result<(), crate::utils::Error> {
        let path = self.uri.split(['?', '#']).next().unwrap_or_default();
        if crate::adapters::http::common::has_dot_segment(path) {
            return Err("target uri holds a dot segment".into());
        }
        Ok(())
    }

    /// Constructs the full URL by combining base_url with uri and query_params
    pub fn full_url(&self) -> Result<String, crate::utils::Error> {
        self.reject_dot_segment_uri()?;

        // Handle double-slash case: if base_url ends with / and uri starts with /, remove one
        let mut url = if self.base_url.ends_with('/') && self.uri.starts_with('/') {
            format!("{}{}", self.base_url.trim_end_matches('/'), self.uri)
        } else {
            format!("{}{}", self.base_url, self.uri)
        };

        // Add query parameters if any exist
        if !self.query_params.is_empty() {
            let params: Vec<String> = self
                .query_params
                .iter()
                .flat_map(|(key, values)| {
                    values.iter().map(move |value| {
                        format!(
                            "{}={}",
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
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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
    /// Content metadata including content-type and parsing status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_metadata: Option<ContentMetadata>,
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
    /// The command that was active when the request was processed.
    /// Forwarded from the RequestEnvelope so middleware right() methods
    /// can inspect the operation without metadata workarounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
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
            command: None,
            original_data,
            normalized_data,
            normalized_snapshot: None,
            local_instance: None,
        }
    }

    /// Creates a new RequestEnvelopeBuilder.
    ///
    /// # Example
    ///
    /// ```text
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
/// ```text
/// let envelope = RequestEnvelopeBuilder::new()
///     .method("POST")
///     .uri("/api/resource")
///     .original_data(vec![1, 2, 3])
///     .build()?;
/// ```
///
/// ## From existing RequestDetails
/// ```text
/// let request_details = RequestDetails { /* ... */ };
/// let envelope = RequestEnvelopeBuilder::from_request_details(request_details)
///     .original_data(serde_json::json!({"key": "value"}))
///     .build()?;
/// ```
///
/// ## With metadata and headers
/// ```text
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
    content_metadata: Option<ContentMetadata>,
    backend_request_details: Option<RequestDetails>,
    target_details: Option<TargetDetails>,
    command: Option<Command>,
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
            content_metadata: None,
            backend_request_details: None,
            target_details: None,
            command: None,
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
            content_metadata: details.content_metadata.clone(),
            backend_request_details: None,
            target_details: None,
            command: None,
            original_data: None,
            normalized_data: None,
            normalized_snapshot: None,
        }
    }

    /// Creates a minimal envelope with only required fields.
    pub fn with_minimal(
        method: impl Into<String>,
        uri: impl Into<String>,
        original_data: T,
    ) -> Self {
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

    /// Sets the content metadata.
    pub fn content_metadata(mut self, content_metadata: Option<ContentMetadata>) -> Self {
        self.content_metadata = content_metadata;
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

    /// Sets the command.
    pub fn command(mut self, command: Option<Command>) -> Self {
        self.command = command;
        self
    }

    /// Sets the command with just an operation.
    pub fn operation(mut self, operation: Operation) -> Self {
        self.command = Some(Command::new(operation));
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
            content_metadata: self.content_metadata,
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
            command: self.command,
            original_data,
            normalized_data,
            normalized_snapshot: self.normalized_snapshot,
            local_instance: None,
        })
    }
}

impl<T> Default for RequestEnvelopeBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ResponseEnvelope<T> {
    /// Creates a new ResponseEnvelope from backend response components without a command.
    ///
    /// **Prefer `from_backend_with_command()`** which propagates the typed Command from
    /// the request pipeline so middleware `right()` methods can inspect the operation
    /// directly instead of relying on metadata strings.
    #[deprecated(
        since = "0.12.3",
        note = "Use from_backend_with_command() to propagate the Command from the request"
    )]
    pub fn from_backend(
        request_details: RequestDetails,
        status: u16,
        headers: HashMap<String, String>,
        original_data: T,
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
            command: None,
            original_data,
            normalized_data: None,
            normalized_snapshot: None,
        }
    }

    /// Creates a new ResponseEnvelope from backend response components with a command.
    pub fn from_backend_with_command(
        request_details: RequestDetails,
        status: u16,
        headers: HashMap<String, String>,
        original_data: T,
        metadata: Option<HashMap<String, String>>,
        command: Option<Command>,
    ) -> Self {
        let response_details = ResponseDetails {
            status,
            headers,
            metadata: metadata.unwrap_or_default(),
        };

        ResponseEnvelope {
            request_details,
            response_details,
            command,
            original_data,
            normalized_data: None,
            normalized_snapshot: None,
        }
    }
}

impl ResponseEnvelope<Vec<u8>> {
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
                    command: self.command,
                    original_data: existing_normalized.clone(),
                    normalized_data: Some(existing_normalized),
                    normalized_snapshot: self.normalized_snapshot,
                });
            }
            // Body is not empty - parse it and preserve normalized_data separately
            // (normalized_data will be preserved below)
        }

        // Check content-type to determine handling strategy
        let content_type = self
            .response_details
            .headers
            .get("content-type")
            .or_else(|| self.response_details.headers.get("Content-Type"))
            .cloned()
            .unwrap_or_default();

        let is_json = is_json_content_type(&content_type);

        // Check for binary content types that need base64 preservation
        // Uses the protocol-agnostic core::content::mime module for comprehensive detection
        let is_binary = !is_json && crate::core::content::mime::is_definitely_binary(&content_type);

        // For binary content, preserve as base64 so middleware can access it
        if is_binary && !self.original_data.is_empty() {
            let binary_wrapper = body_b64_wrapper(&self.original_data);
            return Ok(ResponseEnvelope {
                request_details: self.request_details,
                response_details: self.response_details,
                command: self.command,
                original_data: binary_wrapper.clone(),
                normalized_data: Some(binary_wrapper),
                normalized_snapshot: self.normalized_snapshot,
            });
        }

        // Non-JSON, non-binary bodies (text/html, text/plain, application/xml, ...)
        // are kept as a string so `to_bytes` can write them back verbatim. Bytes
        // that are not valid UTF-8 fall back to the base64 wrapper so they are
        // never altered or dropped. `normalized_data` is left to the middleware.
        if !is_json && !self.original_data.is_empty() {
            let original_json = match std::str::from_utf8(&self.original_data) {
                Ok(text) => serde_json::Value::String(text.to_owned()),
                Err(_) => body_b64_wrapper(&self.original_data),
            };
            return Ok(ResponseEnvelope {
                request_details: self.request_details,
                response_details: self.response_details,
                command: self.command,
                original_data: original_json,
                normalized_data: self.normalized_data,
                normalized_snapshot: self.normalized_snapshot,
            });
        }

        let (parsed_normalized, original_json) = if is_json && !self.original_data.is_empty() {
            match serde_json::from_slice::<serde_json::Value>(&self.original_data) {
                Ok(json) => (Some(json.clone()), json),
                Err(_) => {
                    // The body claims to be JSON but is not (an HTML error page, a
                    // byte order mark). Keep the bytes so the client still gets them.
                    (None, body_b64_wrapper(&self.original_data))
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
            command: self.command,
            original_data: original_json,
            normalized_data: final_normalized,
            normalized_snapshot: self.normalized_snapshot,
        })
    }
}
impl ResponseEnvelope<serde_json::Value> {
    /// Converts back to byte-level envelope, serializing JSON if needed
    pub fn to_bytes(mut self) -> Result<ResponseEnvelope<Vec<u8>>, crate::utils::Error> {
        // Check for body_b64 in normalized_data (binary content preserved by to_json)
        if let Some(ref normalized) = self.normalized_data {
            if let Some(b64) = normalized.get("body_b64").and_then(|v| v.as_str()) {
                use base64::Engine;
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                    return Ok(ResponseEnvelope {
                        request_details: self.request_details,
                        response_details: self.response_details,
                        command: self.command,
                        original_data: bytes,
                        normalized_data: self.normalized_data,
                        normalized_snapshot: self.normalized_snapshot,
                    });
                }
            }
        }

        // Also check original_data for body_b64 (fallback)
        if let Some(b64) = self.original_data.get("body_b64").and_then(|v| v.as_str()) {
            use base64::Engine;
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                return Ok(ResponseEnvelope {
                    request_details: self.request_details,
                    response_details: self.response_details,
                    command: self.command,
                    original_data: bytes,
                    normalized_data: self.normalized_data,
                    normalized_snapshot: self.normalized_snapshot,
                });
            }
        }

        // Prefer normalized_data for body (may have been modified by middleware)
        // Fall back to original_data if normalized_data is None
        let body_bytes = if let Some(text) = self.raw_text_body() {
            text.as_bytes().to_vec()
        } else if let Some(ref normalized) = self.normalized_data {
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
            command: self.command,
            original_data: body_bytes,
            normalized_data: self.normalized_data,
            normalized_snapshot: self.normalized_snapshot,
        })
    }
}

impl ResponseEnvelope<serde_json::Value> {
    /// Returns the body as text when it must be written out verbatim.
    ///
    /// A string body under a non-JSON content type is raw text, not a JSON
    /// string, so quoting it would corrupt the response. When there is no
    /// content type, `normalized_data` still means "serialize as JSON", which
    /// is what middleware-built responses rely on.
    fn raw_text_body(&self) -> Option<&str> {
        let content_type = self
            .response_details
            .headers
            .get("content-type")
            .or_else(|| self.response_details.headers.get("Content-Type"));
        let is_text_context = match content_type {
            Some(ct) => !is_json_content_type(ct),
            None => self.normalized_data.is_none(),
        };
        if !is_text_context {
            return None;
        }
        self.normalized_data
            .as_ref()
            .unwrap_or(&self.original_data)
            .as_str()
    }
}

/// Media types are case-insensitive (RFC 9110), and `is_definitely_binary`
/// already lowercases, so this check must agree with it.
fn is_json_content_type(content_type: &str) -> bool {
    let content_type = content_type.to_ascii_lowercase();
    content_type.contains("application/json")
        || content_type.contains("application/fhir+json")
        || content_type.contains("application/dicom+json")
        || content_type.contains("+json")
}

/// Wraps raw bytes so middleware can carry them through the JSON envelope.
/// `to_bytes` decodes `body_b64` back to the original bytes.
fn body_b64_wrapper(bytes: &[u8]) -> serde_json::Value {
    use base64::Engine;
    serde_json::json!({
        "body_b64": base64::engine::general_purpose::STANDARD.encode(bytes),
        "content_length": bytes.len()
    })
}

/// Command helpers for ResponseEnvelope
impl<T> ResponseEnvelope<T> {
    /// Resolves the operation from the command carried on this response.
    ///
    /// Returns the operation from `envelope.command` if set, otherwise `None`.
    pub fn resolve_operation(&self) -> Option<Operation> {
        self.command.as_ref().map(|cmd| cmd.operation)
    }

    /// Resolves the full command carried on this response.
    ///
    /// Returns a clone of `envelope.command` if set, otherwise `None`.
    pub fn resolve_command(&self) -> Option<Command> {
        self.command.clone()
    }
}

/// Helper methods for modifying target_details in middleware
impl<T> RequestEnvelope<T> {
    /// Gets or initializes target_details from request_details.
    /// This ensures target_details exists before modification.
    fn get_or_init_target_details(&mut self) -> &mut TargetDetails {
        if self.target_details.is_none() {
            self.target_details = Some(TargetDetails {
                base_url: String::new(),
                method: self.request_details.method.clone(),
                uri: self.request_details.uri.clone(),
                headers: self.request_details.headers.clone(),
                cookies: self.request_details.cookies.clone(),
                query_params: self.request_details.query_params.clone(),
                metadata: self.request_details.metadata.clone(),
            });
        }
        self.target_details.as_mut().unwrap()
    }

    /// Sets the base URL for the backend target.
    ///
    /// This allows middleware to override which backend the request is sent to.
    /// If target_details doesn't exist, it will be initialized from request_details.
    ///
    /// # Example
    ///
    /// ```text
    /// envelope.set_target_base_url("https://api.example.com");
    /// ```
    pub fn set_target_base_url(&mut self, base_url: impl Into<String>) {
        let target = self.get_or_init_target_details();
        target.base_url = base_url.into();
    }

    /// Sets the URI/path for the backend target.
    ///
    /// This allows middleware to rewrite the request path before it reaches the backend.
    /// If target_details doesn't exist, it will be initialized from request_details.
    ///
    /// # Example
    ///
    /// ```text
    /// envelope.set_target_uri("/v2/resource");
    /// ```
    pub fn set_target_uri(&mut self, uri: impl Into<String>) {
        let target = self.get_or_init_target_details();
        target.uri = uri.into();
    }

    /// Sets or updates a header in the target_details.
    ///
    /// This allows middleware to add or modify headers that will be sent to the backend.
    /// If target_details doesn't exist, it will be initialized from request_details.
    ///
    /// # Example
    ///
    /// ```text
    /// envelope.set_target_header("Authorization", "Bearer token123");
    /// ```
    pub fn set_target_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let target = self.get_or_init_target_details();
        target.headers.insert(key.into(), value.into());
    }

    /// Sets or updates a query parameter in the target_details.
    ///
    /// This allows middleware to add or modify query parameters sent to the backend.
    /// If target_details doesn't exist, it will be initialized from request_details.
    ///
    /// # Example
    ///
    /// ```text
    /// envelope.set_target_query_param("filter", vec!["active".to_string()]);
    /// ```
    pub fn set_target_query_param(&mut self, key: impl Into<String>, values: Vec<String>) {
        let target = self.get_or_init_target_details();
        target.query_params.insert(key.into(), values);
    }

    /// Sets or updates a metadata entry in the target_details.
    ///
    /// This allows middleware to set backend-specific metadata, such as DIMSE operations.
    /// If target_details doesn't exist, it will be initialized from request_details.
    ///
    /// # Example
    ///
    /// ```text
    /// envelope.set_target_metadata("custom_key", "custom_value");
    /// ```
    pub fn set_target_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let target = self.get_or_init_target_details();
        target.metadata.insert(key.into(), value.into());
    }

    /// Sets the HTTP method for the backend target.
    ///
    /// This allows middleware to change the HTTP method used for the backend request.
    /// If target_details doesn't exist, it will be initialized from request_details.
    ///
    /// # Example
    ///
    /// ```text
    /// envelope.set_target_method("POST");
    /// ```
    pub fn set_target_method(&mut self, method: impl Into<String>) {
        let target = self.get_or_init_target_details();
        target.method = method.into();
    }

    /// Sets the command with just an operation.
    ///
    /// # Example
    ///
    /// ```text
    /// envelope.set_command(Operation::Query);
    /// ```
    pub fn set_command(&mut self, operation: Operation) {
        self.command = Some(Command::new(operation));
    }

    /// Sets the command with operation and parameters.
    ///
    /// # Example
    ///
    /// ```text
    /// let mut params = HashMap::new();
    /// params.insert("query_level".to_string(), json!("STUDY"));
    /// envelope.set_command_with_params(Operation::Query, params);
    /// ```
    pub fn set_command_with_params(
        &mut self,
        operation: Operation,
        params: HashMap<String, serde_json::Value>,
    ) {
        self.command = Some(Command::with_params(operation, params));
    }

    /// Resolves the operation from the command.
    ///
    /// Returns the operation from `envelope.command` if set, otherwise `None`.
    pub fn resolve_operation(&self) -> Option<Operation> {
        self.command.as_ref().map(|cmd| cmd.operation)
    }

    /// Resolves the full command.
    ///
    /// Returns a clone of `envelope.command` if set, otherwise `None`.
    pub fn resolve_command(&self) -> Option<Command> {
        self.command.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_envelope() -> RequestEnvelope<Vec<u8>> {
        RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/test/path")
            .header("content-type", "application/json")
            .query_param("key", vec!["value".to_string()])
            .metadata_entry("protocol", "http")
            .original_data(b"test data".to_vec())
            .build()
            .unwrap()
    }

    fn target_with_uri(uri: &str) -> TargetDetails {
        TargetDetails {
            base_url: "https://api.example.com/fhir".to_string(),
            uri: uri.to_string(),
            ..TargetDetails::default()
        }
    }

    #[test]
    fn full_url_rejects_dot_segments_in_the_path() {
        for uri in [
            "/../admin",
            "/a/./b",
            "/a/%2e%2E/b",
            "/a/.%2e/b",
            "/a\\..\\b",
            "/../admin?x=1",
            "..",
        ] {
            assert!(target_with_uri(uri).full_url().is_err(), "{}", uri);
        }
    }

    #[test]
    fn full_url_allows_dots_outside_path_segments() {
        for uri in [
            "/a.b/c..d",
            "/.well-known/x",
            "/...",
            "/a?next=/../b",
            "/a#/../b",
            "",
        ] {
            let url = target_with_uri(uri).full_url().expect(uri);
            assert_eq!(url, format!("https://api.example.com/fhir{}", uri));
        }
    }

    #[test]
    fn test_set_target_base_url_initializes() {
        let mut envelope = create_test_envelope();
        assert!(envelope.target_details.is_none());

        envelope.set_target_base_url("https://api.example.com");

        assert!(envelope.target_details.is_some());
        let target = envelope.target_details.as_ref().unwrap();
        assert_eq!(target.base_url, "https://api.example.com");
        // Should copy from request_details
        assert_eq!(target.method, "GET");
        assert_eq!(target.uri, "/test/path");
    }

    #[test]
    fn test_set_target_uri() {
        let mut envelope = create_test_envelope();
        envelope.set_target_uri("/new/path");

        let target = envelope.target_details.as_ref().unwrap();
        assert_eq!(target.uri, "/new/path");
        // Other fields should still be copied from request_details
        assert_eq!(target.method, "GET");
    }

    #[test]
    fn test_set_target_header() {
        let mut envelope = create_test_envelope();
        envelope.set_target_header("Authorization", "Bearer token123");

        let target = envelope.target_details.as_ref().unwrap();
        assert_eq!(
            target.headers.get("Authorization"),
            Some(&"Bearer token123".to_string())
        );
        // Should also have original headers
        assert_eq!(
            target.headers.get("content-type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_set_target_query_param() {
        let mut envelope = create_test_envelope();
        envelope.set_target_query_param("filter", vec!["active".to_string()]);

        let target = envelope.target_details.as_ref().unwrap();
        assert_eq!(
            target.query_params.get("filter"),
            Some(&vec!["active".to_string()])
        );
        // Should also have original query params
        assert_eq!(
            target.query_params.get("key"),
            Some(&vec!["value".to_string()])
        );
    }

    #[test]
    fn test_set_target_metadata() {
        let mut envelope = create_test_envelope();
        envelope.set_target_metadata("custom_key", "custom_value");

        let target = envelope.target_details.as_ref().unwrap();
        assert_eq!(
            target.metadata.get("custom_key"),
            Some(&"custom_value".to_string())
        );
        // Should also have original metadata
        assert_eq!(target.metadata.get("protocol"), Some(&"http".to_string()));
    }

    #[test]
    fn test_set_target_method() {
        let mut envelope = create_test_envelope();
        envelope.set_target_method("POST");

        let target = envelope.target_details.as_ref().unwrap();
        assert_eq!(target.method, "POST");
    }

    #[test]
    fn test_chaining_multiple_setters() {
        let mut envelope = create_test_envelope();

        envelope.set_target_base_url("https://backend.example.com");
        envelope.set_target_uri("/v2/api");
        envelope.set_target_method("POST");
        envelope.set_target_header("X-Custom", "value");
        envelope.set_target_metadata("custom", "metadata");

        let target = envelope.target_details.as_ref().unwrap();
        assert_eq!(target.base_url, "https://backend.example.com");
        assert_eq!(target.uri, "/v2/api");
        assert_eq!(target.method, "POST");
        assert_eq!(target.headers.get("X-Custom"), Some(&"value".to_string()));
        assert_eq!(target.metadata.get("custom"), Some(&"metadata".to_string()));
    }

    #[test]
    fn test_update_existing_target_details() {
        let mut envelope = create_test_envelope();

        // Set initial values
        envelope.set_target_base_url("https://first.example.com");
        envelope.set_target_uri("/v1/api");

        // Update values
        envelope.set_target_base_url("https://second.example.com");
        envelope.set_target_uri("/v2/api");

        let target = envelope.target_details.as_ref().unwrap();
        assert_eq!(target.base_url, "https://second.example.com");
        assert_eq!(target.uri, "/v2/api");
    }

    // New tests: binary to_json/to_bytes round trip for application/dicom
    #[test]
    fn test_response_envelope_to_json_binary_dicom_roundtrip() {
        let mut headers = std::collections::HashMap::new();
        headers.insert("content-type".to_string(), "application/dicom".to_string());
        let body = b"DICM_FAKE_BYTES".to_vec();

        let resp = ResponseEnvelope::from_backend_with_command(
            RequestDetails::default(),
            200,
            headers.clone(),
            body.clone(),
            None,
            None,
        );

        // Convert to JSON and ensure body_b64 is present
        let json_env = resp.to_json().expect("to_json failed");
        let nd = json_env
            .normalized_data
            .clone()
            .expect("normalized_data missing");
        assert!(nd.get("body_b64").is_some(), "body_b64 should be present");

        // Convert back to bytes and ensure body matches original
        let bytes_env = json_env.to_bytes().expect("to_bytes failed");
        assert_eq!(bytes_env.original_data, body);
        // Content-type preserved
        assert_eq!(
            bytes_env.response_details.headers.get("content-type"),
            Some(&"application/dicom".to_string())
        );
    }

    // New tests: binary to_json/to_bytes for multipart/related
    #[test]
    fn test_response_envelope_to_json_multipart_roundtrip() {
        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "content-type".to_string(),
            "multipart/related; type=\"application/dicom\"; boundary=abc".to_string(),
        );
        let body = b"--abc\r\nContent-Type: application/dicom\r\n\r\nDATA\r\n--abc--\r\n".to_vec();

        let resp = ResponseEnvelope::from_backend_with_command(
            RequestDetails::default(),
            200,
            headers.clone(),
            body.clone(),
            None,
            None,
        );

        let json_env = resp.to_json().expect("to_json failed");
        let nd = json_env
            .normalized_data
            .clone()
            .expect("normalized_data missing");
        let b64 = nd
            .get("body_b64")
            .and_then(|v| v.as_str())
            .expect("missing b64");
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("decode failed");
        assert_eq!(decoded, body);

        let bytes_env = json_env.to_bytes().expect("to_bytes failed");
        assert_eq!(bytes_env.original_data, body);
    }

    fn text_response(content_type: &str, body: &[u8]) -> ResponseEnvelope<Vec<u8>> {
        let mut headers = std::collections::HashMap::new();
        if !content_type.is_empty() {
            headers.insert("content-type".to_string(), content_type.to_string());
        }
        ResponseEnvelope::from_backend_with_command(
            RequestDetails::default(),
            200,
            headers,
            body.to_vec(),
            None,
            None,
        )
    }

    #[test]
    fn test_response_envelope_text_bodies_roundtrip_verbatim() {
        let cases: [(&str, &[u8]); 4] = [
            ("text/html", b"<html><body>hello</body></html>"),
            ("text/plain; charset=utf-8", b"hello plain"),
            ("application/xml", b"<a><b>hello xml</b></a>"),
            ("text/csv", b"id,name\n1,Alice\n"),
        ];
        for (content_type, body) in cases {
            let json_env = text_response(content_type, body)
                .to_json()
                .expect("to_json failed");
            let bytes_env = json_env.to_bytes().expect("to_bytes failed");
            assert_eq!(
                bytes_env.original_data, body,
                "body lost for content-type {content_type}"
            );
            assert_eq!(
                bytes_env.response_details.headers.get("content-type"),
                Some(&content_type.to_string())
            );
        }
    }

    #[test]
    fn test_response_envelope_text_body_without_content_type_roundtrips() {
        let json_env = text_response("", b"no content type")
            .to_json()
            .expect("to_json failed");
        let bytes_env = json_env.to_bytes().expect("to_bytes failed");
        assert_eq!(bytes_env.original_data, b"no content type");
    }

    #[test]
    fn test_response_envelope_text_body_survives_json_extractor_style_copy() {
        // A middleware that copies original_data into normalized_data (as the
        // json_extractor does on the request side) must not make to_bytes quote
        // the text.
        let mut json_env = text_response("text/html", b"<p>hi</p>")
            .to_json()
            .expect("to_json failed");
        json_env.normalized_data = Some(json_env.original_data.clone());
        let bytes_env = json_env.to_bytes().expect("to_bytes failed");
        assert_eq!(bytes_env.original_data, b"<p>hi</p>");
    }

    #[test]
    fn test_response_envelope_non_utf8_text_body_roundtrips_losslessly() {
        let body: &[u8] = &[0x68, 0xe9, 0x6c, 0x6c, 0xf6]; // latin-1, not valid UTF-8
        let json_env = text_response("text/plain", body)
            .to_json()
            .expect("to_json failed");
        let bytes_env = json_env.to_bytes().expect("to_bytes failed");
        assert_eq!(bytes_env.original_data, body);
    }

    #[test]
    fn test_response_envelope_json_string_body_stays_quoted() {
        let json_env = text_response("application/json", b"\"just a string\"")
            .to_json()
            .expect("to_json failed");
        let bytes_env = json_env.to_bytes().expect("to_bytes failed");
        assert_eq!(bytes_env.original_data, b"\"just a string\"");
    }

    #[test]
    fn test_response_envelope_middleware_object_on_text_response_serializes_as_json() {
        // Documents existing behaviour: an object in normalized_data is written
        // as JSON and the text content type is left unchanged.
        let mut json_env = text_response("text/html", b"<p>hi</p>")
            .to_json()
            .expect("to_json failed");
        json_env.normalized_data = Some(serde_json::json!({"rewritten": true}));
        let bytes_env = json_env.to_bytes().expect("to_bytes failed");
        assert_eq!(bytes_env.original_data, br#"{"rewritten":true}"#);
    }

    #[test]
    fn test_response_envelope_to_json_holds_text_as_string_and_keeps_headers() {
        let json_env = text_response("text/html", b"<p>hi</p>")
            .to_json()
            .expect("to_json failed");
        assert_eq!(json_env.original_data, serde_json::json!("<p>hi</p>"));
        assert!(
            json_env.normalized_data.is_none(),
            "text must not be offered to middleware as normalized JSON"
        );
        assert_eq!(json_env.response_details.status, 200);
        assert_eq!(
            json_env.response_details.headers.get("content-type"),
            Some(&"text/html".to_string())
        );
    }

    #[test]
    fn test_response_envelope_to_json_wraps_non_utf8_text_as_body_b64() {
        let json_env = text_response("text/plain", &[0x68, 0xe9])
            .to_json()
            .expect("to_json failed");
        assert!(json_env.original_data.get("body_b64").is_some());
    }

    #[test]
    fn test_response_envelope_empty_text_bodies_stay_empty() {
        // HEAD, 204 and 304 responses carry a content type and no body.
        for status in [200u16, 204, 304] {
            let mut resp = text_response("text/html", b"");
            resp.response_details.status = status;
            let bytes_env = resp
                .to_json()
                .expect("to_json failed")
                .to_bytes()
                .expect("to_bytes failed");
            assert!(bytes_env.original_data.is_empty(), "status {status}");
            assert_eq!(bytes_env.response_details.status, status);
            assert_eq!(
                bytes_env.response_details.headers.get("content-type"),
                Some(&"text/html".to_string())
            );
        }
    }

    #[test]
    fn test_response_envelope_text_error_bodies_roundtrip() {
        for status in [404u16, 500, 502] {
            let mut resp = text_response("text/html", b"<h1>upstream error</h1>");
            resp.response_details.status = status;
            let bytes_env = resp
                .to_json()
                .expect("to_json failed")
                .to_bytes()
                .expect("to_bytes failed");
            assert_eq!(bytes_env.original_data, b"<h1>upstream error</h1>");
            assert_eq!(bytes_env.response_details.status, status);
        }
    }

    #[test]
    fn test_response_envelope_invalid_json_body_is_preserved() {
        let cases: [(&str, &[u8]); 3] = [
            ("application/json", b"<html>502 Bad Gateway</html>"),
            ("application/json", b"\xef\xbb\xbf{\"bom\":true}"),
            ("application/fhir+json", b"not json at all"),
        ];
        for (content_type, body) in cases {
            let bytes_env = text_response(content_type, body)
                .to_json()
                .expect("to_json failed")
                .to_bytes()
                .expect("to_bytes failed");
            assert_eq!(
                bytes_env.original_data, body,
                "body lost for content-type {content_type}"
            );
        }
    }

    #[test]
    fn test_response_envelope_content_type_match_is_case_insensitive() {
        let json_env = text_response("Application/JSON", br#"{"a":1}"#)
            .to_json()
            .expect("to_json failed");
        assert_eq!(
            json_env.normalized_data,
            Some(serde_json::json!({"a": 1})),
            "mixed-case JSON content type must still be parsed"
        );

        let bytes_env = text_response("TEXT/HTML", b"<p>hi</p>")
            .to_json()
            .expect("to_json failed")
            .to_bytes()
            .expect("to_bytes failed");
        assert_eq!(bytes_env.original_data, b"<p>hi</p>");
    }
}
