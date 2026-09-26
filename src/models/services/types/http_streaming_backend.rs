use crate::config::config::ConfigError;
use crate::models::envelope::command::Command;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::models::services::streaming::{ResponseStream, StreamingBackend};
use crate::models::services::upstream_headers::{
    upstream_request_headers, InboundAuthorizationPolicy,
};
use async_trait::async_trait;
use axum::response::Response;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;

use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use tracing::{debug, info, warn};

/// HTTP streaming backend configuration
///
/// This service consumes HTTP streams (SSE, NDJSON, or multipart) from a remote
/// endpoint and reconstructs them into a stream of ResponseEnvelopes.
///
/// Use Cases:
/// - Receive streamed DICOM instances from another Harmony proxy
/// - Consume database query streams over HTTP
/// - Receive any streamed data for processing/forwarding
///
/// Supported Formats:
/// - **SSE (Server-Sent Events)**: Parses `text/event-stream` format
/// - **NDJSON (Newline-Delimited JSON)**: Parses `application/x-ndjson` format
/// - **Multipart**: Parses `multipart/x-mixed-replace` format
#[derive(Debug, Deserialize)]
pub struct HttpStreamingBackend {
    /// Base URL of the streaming endpoint
    pub base_url: Option<String>,
    
    /// Expected streaming format (auto-detect if not specified)
    pub format: Option<StreamFormat>,
    
    /// Timeout for receiving each chunk (seconds)
    pub chunk_timeout_secs: Option<u64>,
    
    /// Maximum chunk size (bytes)
    pub max_chunk_size: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StreamFormat {
    /// Server-Sent Events (text/event-stream)
    Sse,
    /// Newline-Delimited JSON (application/x-ndjson)
    Ndjson,
    /// Multipart messages (multipart/x-mixed-replace)
    Multipart,
    /// Auto-detect from Content-Type header
    Auto,
}

impl Default for StreamFormat {
    fn default() -> Self {
        Self::Auto
    }
}

impl StreamFormat {
    /// Detect format from Content-Type header
    pub fn from_content_type(content_type: &str) -> Self {
        let ct_lower = content_type.to_lowercase();
        
        if ct_lower.contains("text/event-stream") {
            Self::Sse
        } else if ct_lower.contains("application/x-ndjson") || ct_lower.contains("application/ndjson") {
            Self::Ndjson
        } else if ct_lower.contains("multipart/") {
            Self::Multipart
        } else {
            // Try NDJSON as fallback
            Self::Ndjson
        }
    }
}

#[async_trait]
impl ServiceType for HttpStreamingBackend {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate base_url
        let base_url = options
            .get("base_url")
            .and_then(|v| v.as_str())
            .or(self.base_url.as_deref());
            
        if base_url.is_none() {
            return Err(ConfigError::InvalidEndpoint {
                name: "http_streaming_backend".to_string(),
                reason: "base_url is required".to_string(),
            });
        }

        // Validate chunk_timeout if provided
        if let Some(timeout) = self.chunk_timeout_secs {
            if timeout == 0 || timeout > 3600 {
                return Err(ConfigError::InvalidEndpoint {
                    name: "http_streaming_backend".to_string(),
                    reason: "chunk_timeout_secs must be between 1 and 3600".to_string(),
                });
            }
        }

        Ok(())
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Backend usage only - no HTTP routes
        vec![]
    }

    async fn build_protocol_envelope(
        &self,
        _ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "HttpStreamingBackend doesn't support protocol envelope building",
        ))
    }
}

#[async_trait]
impl ServiceHandler<Value> for HttpStreamingBackend {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        _envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "HttpStreamingBackend is a backend, not an endpoint",
        ))
    }

    async fn backend_outgoing_request(
        &self,
        _envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "HttpStreamingBackend requires streaming execution - use execute_streaming()",
        ))
    }

    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        _ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        envelope
            .response_details
            .metadata
            .insert("service".to_string(), "http_streaming_backend".to_string());
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        _envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        Err(Error::from(
            "HttpStreamingBackend doesn't support endpoint_outgoing_response",
        ))
    }
}

#[async_trait]
impl StreamingBackend for HttpStreamingBackend {
    async fn backend_outgoing_stream(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseStream, Error> {
        info!("Starting HTTP streaming backend request");

        // Get base URL
        let base_url = options
            .get("base_url")
            .and_then(|v| v.as_str())
            .or(self.base_url.as_deref())
            .ok_or_else(|| Error::from("base_url is required"))?;

        // Get path prefix from options
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Build request URL - combine base_url + path_prefix + path from metadata
        let path = envelope.request_details.metadata
            .get("path")
            .map(|p| format!("/{}", p))
            .unwrap_or_default();
        let url = format!("{}{}{}", base_url, path_prefix, path);

        info!("Streaming from URL: {}", url);

        // Build HTTP client
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // 5 minute total timeout
            .build()
            .map_err(|e| Error::from(format!("Failed to create HTTP client: {}", e)))?;

        // Determine HTTP method
        let method = envelope.request_details.method.as_str();
        
        // Build request
        let mut request = match method.to_uppercase().as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url).body(envelope.original_data.clone()),
            "PUT" => client.put(&url).body(envelope.original_data.clone()),
            _ => return Err(Error::from(format!("Unsupported HTTP method: {}", method))),
        };

        // This backend sends the client's headers (it does not read
        // target_details), so every candidate header is an inbound one.
        for (key, value) in upstream_request_headers(
            &envelope.request_details.headers,
            &envelope.request_details.headers,
            InboundAuthorizationPolicy::from_backend_options(options)?,
            "HTTP streaming",
        ) {
            request = request.header(key, value);
        }

        // Add Accept header based on expected format
        let format = self.format.unwrap_or(StreamFormat::Auto);
        if matches!(format, StreamFormat::Sse) {
            request = request.header("Accept", "text/event-stream");
        } else if matches!(format, StreamFormat::Ndjson) {
            request = request.header("Accept", "application/x-ndjson");
        } else if matches!(format, StreamFormat::Multipart) {
            request = request.header("Accept", "multipart/x-mixed-replace");
        }

        // Execute request
        let response = request
            .send()
            .await
            .map_err(|e| Error::from(format!("HTTP request failed: {}", e)))?;

        // Check status
        let status = response.status();
        if !status.is_success() {
            return Err(Error::from(format!("HTTP request failed with status: {}", status)));
        }

        // Detect format from Content-Type
        let detected_format = if matches!(format, StreamFormat::Auto) {
            response
                .headers()
                .get("content-type")
                .and_then(|ct| ct.to_str().ok())
                .map(StreamFormat::from_content_type)
                .unwrap_or(StreamFormat::Ndjson)
        } else {
            format
        };

        info!("Detected streaming format: {:?}", detected_format);

        // Extract boundary before consuming response (for multipart)
        let boundary_opt = if matches!(detected_format, StreamFormat::Multipart) {
            response
                .headers()
                .get("content-type")
                .and_then(|ct| ct.to_str().ok())
                .and_then(Self::extract_boundary)
        } else {
            None
        };

        // Get byte stream from response
        let byte_stream = response.bytes_stream();

        // Convert to ResponseEnvelope stream based on format
        let request_details = envelope.request_details.clone();
        let command = envelope.command.clone();
        let envelope_stream = match detected_format {
            StreamFormat::Sse => Self::parse_sse_stream(byte_stream, request_details, command),
            StreamFormat::Ndjson => Self::parse_ndjson_stream(byte_stream, request_details, command),
            StreamFormat::Multipart => {
                let boundary = boundary_opt
                    .ok_or_else(|| Error::from("Missing boundary in multipart Content-Type"))?;
                
                Self::parse_multipart_stream(byte_stream, request_details, boundary, command)
            }
            StreamFormat::Auto => {
                // Should not reach here, but fallback to NDJSON
                Self::parse_ndjson_stream(byte_stream, request_details, command)
            }
        };

        Ok(Box::pin(envelope_stream))
    }
}

impl HttpStreamingBackend {
    /// Parse SSE stream into ResponseEnvelopes
    fn parse_sse_stream(
        byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
        request_details: crate::models::envelope::envelope::RequestDetails,
        command: Option<Command>,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> + Send>> {
        let mut buffer = String::new();
        let mut sequence = 0u64;

        let stream = byte_stream.flat_map(move |result| {
            match result {
                Ok(bytes) => {
                    // Append to buffer
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    // Split by double newline (SSE event separator)
                    let mut envelopes = Vec::new();
                    
                    while let Some(pos) = buffer.find("\n\n") {
                        let event = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        // Parse SSE event
                        if let Some(envelope) = Self::parse_sse_event(&event, &request_details, &mut sequence, &command) {
                            envelopes.push(Ok(envelope));
                        }
                    }

                    futures_util::stream::iter(envelopes)
                }
                Err(e) => {
                    futures_util::stream::iter(vec![Err(Error::from(format!("Stream error: {}", e)))])
                }
            }
        });
        
        Box::pin(stream)
    }

    /// Parse a single SSE event
    fn parse_sse_event(
        event: &str,
        request_details: &crate::models::envelope::envelope::RequestDetails,
        sequence: &mut u64,
        command: &Option<Command>,
    ) -> Option<ResponseEnvelope<Vec<u8>>> {
        let mut event_type = "message";
        let mut data = String::new();

        for line in event.lines() {
            if line.starts_with("event:") {
                event_type = line[6..].trim();
            } else if line.starts_with("data:") {
                data = line[5..].trim().to_string();
            }
        }

        if data.is_empty() {
            return None;
        }

        // Handle error events
        if event_type == "error" {
            warn!("SSE error event: {}", data);
            return None;
        }

        *sequence += 1;

        // Parse JSON data
        match serde_json::from_str::<serde_json::Value>(&data) {
            Ok(json) => {
                let body = serde_json::to_vec(&json).unwrap_or_default();
                
                let mut metadata = HashMap::new();
                metadata.insert("stream_sequence".to_string(), sequence.to_string());
                metadata.insert("event_type".to_string(), event_type.to_string());

                let mut response = ResponseEnvelope::from_backend_with_command(
                    request_details.clone(),
                    200,
                    HashMap::from([("content-type".to_string(), "application/json".to_string())]),
                    body,
                    Some(metadata),
                    command.clone(),
                );

                response.normalized_data = Some(json);
                
                debug!("Parsed SSE event #{}: {} bytes", sequence, response.original_data.len());
                Some(response)
            }
            Err(e) => {
                warn!("Failed to parse SSE data as JSON: {}", e);
                None
            }
        }
    }

    /// Parse NDJSON stream into ResponseEnvelopes
    /// 
    /// Also handles JSON array responses - if the entire content is a JSON array,
    /// each element is treated as a separate item.
    pub(crate) fn parse_ndjson_stream(
        byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
        request_details: crate::models::envelope::envelope::RequestDetails,
        command: Option<Command>,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> + Send>> {
        let mut buffer = String::new();
        let mut sequence = 0u64;
        let mut processed_as_array = false;

        let stream = byte_stream.flat_map(move |result| {
            match result {
                Ok(bytes) => {
                    // Append to buffer
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    let mut envelopes = Vec::new();
                    
                    // First try to parse NDJSON (newline-delimited)
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if line.trim().is_empty() {
                            continue;
                        }

                        sequence += 1;

                        // Parse JSON line
                        match serde_json::from_str::<serde_json::Value>(&line) {
                            Ok(json) => {
                                if let Some(envelope) = Self::json_to_envelope(&json, &request_details, sequence, &command) {
                                    envelopes.push(Ok(envelope));
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse NDJSON line: {}", e);
                            }
                        }
                    }

                    // If no newlines found, buffer might contain a complete JSON array response
                    // Try to parse it when it looks complete (ends with ] or })
                    if envelopes.is_empty() && !processed_as_array {
                        let trimmed = buffer.trim();
                        if (trimmed.starts_with('[') && trimmed.ends_with(']')) ||
                           (trimmed.starts_with('{') && trimmed.ends_with('}')) {
                            // Try to parse as JSON
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                processed_as_array = true;
                                
                                // If it's an array, treat each element as a separate item
                                if let Some(array) = json.as_array() {
                                    debug!("Parsing JSON array with {} elements", array.len());
                                    for item in array {
                                        sequence += 1;
                                        if let Some(envelope) = Self::json_to_envelope(item, &request_details, sequence, &command) {
                                            envelopes.push(Ok(envelope));
                                        }
                                    }
                                } else {
                                    // Single object
                                    sequence += 1;
                                    if let Some(envelope) = Self::json_to_envelope(&json, &request_details, sequence, &command) {
                                        envelopes.push(Ok(envelope));
                                    }
                                }
                                buffer.clear();
                            }
                        }
                    }

                    futures_util::stream::iter(envelopes)
                }
                Err(e) => {
                    futures_util::stream::iter(vec![Err(Error::from(format!("Stream error: {}", e)))])
                }
            }
        });
        
        Box::pin(stream)
    }

    /// Convert a JSON value to a ResponseEnvelope
    fn json_to_envelope(
        json: &serde_json::Value,
        request_details: &crate::models::envelope::envelope::RequestDetails,
        sequence: u64,
        command: &Option<Command>,
    ) -> Option<ResponseEnvelope<Vec<u8>>> {
        // Check for error marker
        if let Some(error) = json.get("_error") {
            warn!("JSON error: {}", error);
            return None;
        }

        let body = serde_json::to_vec(json).unwrap_or_default();
        
        let mut metadata = HashMap::new();
        metadata.insert("stream_sequence".to_string(), sequence.to_string());
        
        // Extract SOP Instance UID if present
        if let Some(sop_uid) = json.get("_sop_instance_uid").and_then(|v| v.as_str()) {
            metadata.insert("sop_instance_uid".to_string(), sop_uid.to_string());
        }

        let mut response = ResponseEnvelope::from_backend_with_command(
            request_details.clone(),
            200,
            HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body,
            Some(metadata),
            command.clone(),
        );

        response.normalized_data = Some(json.clone());
        
        debug!("Parsed JSON item #{}: {} bytes", sequence, response.original_data.len());
        Some(response)
    }

    /// Parse multipart stream into ResponseEnvelopes
    fn parse_multipart_stream(
        byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
        request_details: crate::models::envelope::envelope::RequestDetails,
        boundary: String,
        command: Option<Command>,
    ) -> Pin<Box<dyn Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> + Send>> {
        let mut buffer = Vec::new();
        let boundary_marker = format!("--{}", boundary);
        let mut sequence = 0u64;

        let stream = byte_stream.flat_map(move |result| {
            match result {
                Ok(bytes) => {
                    // Append to buffer
                    buffer.extend_from_slice(&bytes);

                    let mut envelopes = Vec::new();
                    
                    // Look for boundary markers
                    while let Some(part) = Self::extract_multipart_part(&mut buffer, &boundary_marker) {
                        sequence += 1;

                        if let Some(envelope) = Self::parse_multipart_part(&part, &request_details, sequence, &command) {
                            debug!("Parsed multipart part #{}: {} bytes", sequence, envelope.original_data.len());
                            envelopes.push(Ok(envelope));
                        }
                    }

                    futures_util::stream::iter(envelopes)
                }
                Err(e) => {
                    futures_util::stream::iter(vec![Err(Error::from(format!("Stream error: {}", e)))])
                }
            }
        });
        
        Box::pin(stream)
    }

    /// Extract a single multipart part from buffer
    fn extract_multipart_part(buffer: &mut Vec<u8>, boundary: &str) -> Option<Vec<u8>> {
        let boundary_bytes = boundary.as_bytes();
        
        // Find first boundary
        let start_pos = buffer.windows(boundary_bytes.len())
            .position(|window| window == boundary_bytes)?;
        
        // Skip past the boundary and CRLF
        let content_start = start_pos + boundary_bytes.len() + 2; // +2 for \r\n
        
        if content_start >= buffer.len() {
            return None;
        }

        // Find next boundary
        let next_boundary_pos = buffer[content_start..]
            .windows(boundary_bytes.len())
            .position(|window| window == boundary_bytes)?;
        
        let content_end = content_start + next_boundary_pos;
        
        // Extract part
        let part = buffer[content_start..content_end].to_vec();
        
        // Remove processed data from buffer
        buffer.drain(..content_end);
        
        Some(part)
    }

    /// Parse a single multipart part
    fn parse_multipart_part(
        part: &[u8],
        request_details: &crate::models::envelope::envelope::RequestDetails,
        sequence: u64,
        command: &Option<Command>,
    ) -> Option<ResponseEnvelope<Vec<u8>>> {
        // Split headers and body by double CRLF
        let part_str = String::from_utf8_lossy(part);
        let mut parts = part_str.splitn(2, "\r\n\r\n");
        
        let headers_str = parts.next()?;
        let body_start = headers_str.len() + 4; // +4 for \r\n\r\n
        
        if body_start >= part.len() {
            return None;
        }
        
        // Parse headers
        let mut headers = HashMap::new();
        let mut metadata = HashMap::new();
        
        for line in headers_str.lines() {
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim().to_lowercase();
                let value = value.trim().to_string();
                
                if key.starts_with("x-") {
                    // Custom header -> metadata
                    let meta_key = key.strip_prefix("x-").unwrap_or(&key);
                    metadata.insert(meta_key.replace('-', "_"), value);
                } else {
                    headers.insert(key, value);
                }
            }
        }
        
        metadata.insert("stream_sequence".to_string(), sequence.to_string());
        
        // Extract body
        let body = part[body_start..].to_vec();
        
        let response = ResponseEnvelope::from_backend_with_command(
            request_details.clone(),
            200,
            headers,
            body,
            Some(metadata),
            command.clone(),
        );
        
        Some(response)
    }

    /// Extract boundary from Content-Type header
    fn extract_boundary(content_type: &str) -> Option<String> {
        content_type
            .split(';')
            .find_map(|part| {
                let part = part.trim();
                if part.starts_with("boundary=") {
                    Some(part[9..].trim_matches('"').to_string())
                } else {
                    None
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_format_from_content_type() {
        assert_eq!(
            StreamFormat::from_content_type("text/event-stream"),
            StreamFormat::Sse
        );
        assert_eq!(
            StreamFormat::from_content_type("application/x-ndjson"),
            StreamFormat::Ndjson
        );
        assert_eq!(
            StreamFormat::from_content_type("multipart/x-mixed-replace; boundary=abc"),
            StreamFormat::Multipart
        );
    }

    #[test]
    fn test_extract_boundary() {
        assert_eq!(
            HttpStreamingBackend::extract_boundary("multipart/x-mixed-replace; boundary=harmony-stream-123"),
            Some("harmony-stream-123".to_string())
        );
        assert_eq!(
            HttpStreamingBackend::extract_boundary("multipart/x-mixed-replace; boundary=\"harmony-stream-123\""),
            Some("harmony-stream-123".to_string())
        );
        assert_eq!(
            HttpStreamingBackend::extract_boundary("application/json"),
            None
        );
    }
}
