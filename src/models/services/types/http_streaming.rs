use crate::config::config::ConfigError;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use async_trait::async_trait;
use axum::response::Response;
use axum::body::Body;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;

use crate::router::route_config::{standard_methods, RouteConfig};
use crate::utils::Error;
use tracing::{debug, info};

/// HTTP streaming endpoint configuration
///
/// This service accepts a stream of ResponseEnvelopes and converts them to
/// HTTP streaming formats (SSE, NDJSON, or multipart).
///
/// Use Cases:
/// - Stream DICOM instances to HTTP clients as they arrive from PACS
/// - Stream database query results progressively
/// - Stream file listings or search results
///
/// Supported Formats:
/// - **SSE (Server-Sent Events)**: `text/event-stream` - Good for JSON data in browsers
/// - **NDJSON (Newline-Delimited JSON)**: `application/x-ndjson` - Efficient for line-based processing
/// - **Multipart**: `multipart/x-mixed-replace` - For binary data like DICOM instances
#[derive(Debug, Deserialize)]
pub struct HttpStreamingEndpoint {
    /// Default streaming format if not specified by Accept header
    pub default_format: Option<StreamFormat>,
    
    /// Buffer size for the stream (number of envelopes to buffer)
    pub buffer_size: Option<usize>,
    
    /// Enable compression (gzip) for the stream
    pub enable_compression: Option<bool>,
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
}

impl Default for StreamFormat {
    fn default() -> Self {
        Self::Ndjson
    }
}

impl StreamFormat {
    /// Parse format from configuration string
    pub fn from_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "sse" => Self::Sse,
            "ndjson" => Self::Ndjson,
            "multipart" => Self::Multipart,
            _ => Self::Ndjson, // Default
        }
    }
    
    /// Detect format from Accept header
    pub fn from_accept_header(accept: &str) -> Self {
        let accept_lower = accept.to_lowercase();
        
        if accept_lower.contains("text/event-stream") {
            Self::Sse
        } else if accept_lower.contains("application/x-ndjson") || accept_lower.contains("application/ndjson") {
            Self::Ndjson
        } else if accept_lower.contains("multipart/") {
            Self::Multipart
        } else {
            // Default to NDJSON for compatibility
            Self::Ndjson
        }
    }
    
    /// Get content type for this format
    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Sse => "text/event-stream",
            Self::Ndjson => "application/x-ndjson",
            Self::Multipart => "multipart/x-mixed-replace",
        }
    }
}

#[async_trait]
impl ServiceType for HttpStreamingEndpoint {
    fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate buffer_size if provided
        if let Some(size) = self.buffer_size {
            if size == 0 || size > 10000 {
                return Err(ConfigError::InvalidEndpoint {
                    name: "http_streaming".to_string(),
                    reason: "buffer_size must be between 1 and 10000".to_string(),
                });
            }
        }
        
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Register a route so the HTTP adapter knows to forward requests here.
        // Uses `path_prefix` option (default "/"), matching the HttpEndpoint convention.
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("/")
            .to_string();

        let prefix_trimmed = path_prefix.trim_end_matches('/');
        let wildcard_path = if prefix_trimmed.is_empty() {
            "/{*wildcard}".to_string()
        } else {
            format!("{}/{{*wildcard}}", prefix_trimmed)
        };

        let methods = standard_methods();

        vec![
            RouteConfig {
                path: path_prefix.clone(),
                methods: methods.clone(),
                description: Some("HTTP streaming endpoint (exact path)".to_string()),
            },
            RouteConfig {
                path: wildcard_path,
                methods,
                description: Some("HTTP streaming endpoint (subpaths)".to_string()),
            },
        ]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Only the response of a streaming endpoint is streamed. The request is an
        // ordinary HTTP request, so delegate to HttpEndpoint for consistent parsing.
        if ctx.protocol == crate::models::protocol::Protocol::Http {
            let http = crate::models::services::types::http::HttpEndpoint {};
            return http.build_protocol_envelope(ctx, options).await;
        }
        Err(Error::from(
            "HttpStreamingEndpoint only supports Protocol::Http envelope building",
        ))
    }
}

#[async_trait]
impl ServiceHandler<Value> for HttpStreamingEndpoint {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Pass through - no preprocessing needed
        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        _envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "HttpStreamingEndpoint is an endpoint, not a backend",
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
            .insert("service".to_string(), "http_streaming".to_string());
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // This won't be used for streaming - adapter should handle stream conversion
        // For non-streaming fallback, just return the envelope as JSON
        let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        for (k, v) in &envelope.response_details.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else if let Some(normalized) = envelope.normalized_data {
            let body_bytes = serde_json::to_vec(&normalized)
                .map_err(|_| Error::from("Failed to serialize response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct HTTP response"))
    }
}

impl HttpStreamingEndpoint {
    /// Convert a stream of ResponseEnvelopes to an HTTP streaming response
    ///
    /// This is the core method that adapters will call to convert the ResponseEnvelope
    /// stream into an HTTP response with appropriate streaming format.
    ///
    /// # Arguments
    /// * `envelope_stream` - Stream of ResponseEnvelopes to send
    /// * `format` - Streaming format (SSE, NDJSON, or Multipart)
    /// * `request_headers` - Original request headers (for detecting preferences)
    ///
    /// # Returns
    /// HTTP Response with streaming body
    pub fn build_streaming_response(
        &self,
        envelope_stream: Pin<Box<dyn Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> + Send>>,
        format: StreamFormat,
        _request_headers: &HashMap<String, String>,
    ) -> Result<Response, Error> {
        info!("Building streaming response with format: {:?}", format);

        match format {
            StreamFormat::Sse => self.build_sse_response(envelope_stream),
            StreamFormat::Ndjson => self.build_ndjson_response(envelope_stream),
            StreamFormat::Multipart => self.build_multipart_response(envelope_stream),
        }
    }

    /// Build Server-Sent Events response
    fn build_sse_response(
        &self,
        envelope_stream: Pin<Box<dyn Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> + Send>>,
    ) -> Result<Response, Error> {
        let mut sequence = 0u64;

        let sse_stream = envelope_stream.map(move |result| {
            sequence += 1;

            match result {
                Ok(envelope) => {
                    // Extract JSON data for SSE
                    let json = envelope.normalized_data
                        .unwrap_or(serde_json::Value::Null);

                    // Format as SSE event
                    // SSE format: "event: message\nid: N\ndata: {json}\n\n"
                    let event_type = envelope.response_details.metadata
                        .get("event_type")
                        .map(|s| s.as_str())
                        .unwrap_or("message");

                    let sse_event = format!(
                        "event: {}\nid: {}\ndata: {}\n\n",
                        event_type,
                        sequence,
                        serde_json::to_string(&json).unwrap_or_else(|_| "null".to_string())
                    );

                    debug!("SSE event #{}: {} bytes", sequence, sse_event.len());
                    Ok::<_, Error>(Bytes::from(sse_event))
                }
                Err(e) => {
                    // Send error as SSE event
                    let error_event = format!(
                        "event: error\nid: {}\ndata: {{\"error\": \"{}\"}}\n\n",
                        sequence,
                        e.to_string().replace('"', "\\\"")
                    );
                    Ok(Bytes::from(error_event))
                }
            }
        });

        let body = Body::from_stream(sse_stream);

        Response::builder()
            .status(200)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no") // Disable nginx buffering
            .header("connection", "keep-alive")
            .body(body)
            .map_err(|_| Error::from("Failed to build SSE response"))
    }

    /// Build Newline-Delimited JSON response
    fn build_ndjson_response(
        &self,
        envelope_stream: Pin<Box<dyn Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> + Send>>,
    ) -> Result<Response, Error> {
        let mut sequence = 0u64;

        let ndjson_stream = envelope_stream.map(move |result| {
            sequence += 1;

            match result {
                Ok(envelope) => {
                    // Extract JSON data
                    let json = envelope.normalized_data
                        .unwrap_or(serde_json::Value::Null);

                    // Add metadata if available
                    let mut output = json.clone();
                    if let Some(obj) = output.as_object_mut() {
                        // Add stream metadata
                        obj.insert("_stream_sequence".to_string(), serde_json::json!(sequence));
                        
                        if let Some(sop_uid) = envelope.response_details.metadata.get("sop_instance_uid") {
                            obj.insert("_sop_instance_uid".to_string(), serde_json::json!(sop_uid));
                        }
                    }

                    // Format as NDJSON (JSON + newline)
                    let line = format!(
                        "{}\n",
                        serde_json::to_string(&output).unwrap_or_else(|_| "null".to_string())
                    );

                    debug!("NDJSON line #{}: {} bytes", sequence, line.len());
                    Ok::<_, Error>(Bytes::from(line))
                }
                Err(e) => {
                    // Send error as NDJSON line
                    let error_line = format!(
                        "{{\"_error\": \"{}\", \"_stream_sequence\": {}}}\n",
                        e.to_string().replace('"', "\\\""),
                        sequence
                    );
                    Ok(Bytes::from(error_line))
                }
            }
        });

        let body = Body::from_stream(ndjson_stream);

        Response::builder()
            .status(200)
            .header("content-type", "application/x-ndjson")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(body)
            .map_err(|_| Error::from("Failed to build NDJSON response"))
    }

    /// Build Multipart response
    fn build_multipart_response(
        &self,
        envelope_stream: Pin<Box<dyn Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> + Send>>,
    ) -> Result<Response, Error> {
        // Generate unique boundary
        let boundary = format!("harmony-stream-{}", uuid::Uuid::new_v4());
        let boundary_for_parts = boundary.clone();
        let boundary_for_close = boundary.clone();

        let mut sequence = 0u64;

        let multipart_stream = envelope_stream.map(move |result| {
            sequence += 1;

            match result {
                Ok(envelope) => {
                    // Determine content type from envelope
                    let content_type = envelope.response_details.headers
                        .get("content-type")
                        .or_else(|| envelope.response_details.headers.get("Content-Type"))
                        .cloned()
                        .unwrap_or_else(|| "application/octet-stream".to_string());

                    // Build multipart section
                    let mut part = String::new();
                    
                    // Add leading newline for all except first (sequence > 1)
                    if sequence > 1 {
                        part.push_str("\r\n");
                    }
                    
                    part.push_str(&format!("--{}\r\n", boundary_for_parts));
                    part.push_str(&format!("Content-Type: {}\r\n", content_type));
                    
                    // Add custom headers
                    if let Some(sop_uid) = envelope.response_details.metadata.get("sop_instance_uid") {
                        part.push_str(&format!("X-DICOM-SOP-Instance-UID: {}\r\n", sop_uid));
                    }
                    part.push_str(&format!("X-Stream-Sequence: {}\r\n", sequence));
                    part.push_str("\r\n");

                    debug!("Multipart section #{}: {} + {} bytes", 
                           sequence, part.len(), envelope.original_data.len());

                    // Combine header and body
                    let mut bytes = Vec::with_capacity(part.len() + envelope.original_data.len());
                    bytes.extend_from_slice(part.as_bytes());
                    bytes.extend_from_slice(&envelope.original_data);

                    Ok::<_, Error>(Bytes::from(bytes))
                }
                Err(e) => {
                    // Send error as text part
                    let error_part = format!(
                        "\r\n--{}\r\nContent-Type: application/json\r\n\r\n{{\"error\": \"{}\"}}\r\n",
                        boundary_for_parts,
                        e.to_string().replace('"', "\\\"")
                    );
                    Ok(Bytes::from(error_part))
                }
            }
        });

        // Add closing boundary at the end
        let final_stream = multipart_stream.chain(futures_util::stream::once(async move {
            Ok::<_, Error>(Bytes::from(format!("\r\n--{}--\r\n", boundary_for_close)))
        }));

        let body = Body::from_stream(final_stream);

        Response::builder()
            .status(200)
            .header("content-type", format!("multipart/x-mixed-replace; boundary={}", boundary))
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(body)
            .map_err(|_| Error::from("Failed to build multipart response"))
    }

    /// Determine streaming format from request
    ///
    /// Priority:
    /// 1. Explicit format in options
    /// 2. Accept header
    /// 3. Default format from config
    /// 4. NDJSON (fallback)
    pub fn determine_format(
        &self,
        request_headers: &HashMap<String, String>,
        options: &HashMap<String, Value>,
    ) -> StreamFormat {
        // Check options first
        if let Some(format_str) = options.get("format").and_then(|v| v.as_str()) {
            return match format_str.to_lowercase().as_str() {
                "sse" => StreamFormat::Sse,
                "ndjson" => StreamFormat::Ndjson,
                "multipart" => StreamFormat::Multipart,
                _ => self.default_format.unwrap_or_default(),
            };
        }

        // Check Accept header
        if let Some(accept) = request_headers.get("accept").or_else(|| request_headers.get("Accept")) {
            return StreamFormat::from_accept_header(accept);
        }

        // Use default or fallback
        self.default_format.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Method;

    #[test]
    fn test_stream_format_from_accept() {
        assert_eq!(StreamFormat::from_accept_header("text/event-stream"), StreamFormat::Sse);
        assert_eq!(StreamFormat::from_accept_header("application/x-ndjson"), StreamFormat::Ndjson);
        assert_eq!(StreamFormat::from_accept_header("multipart/mixed"), StreamFormat::Multipart);
        assert_eq!(StreamFormat::from_accept_header("application/json"), StreamFormat::Ndjson);
    }

    #[test]
    fn test_stream_format_content_type() {
        assert_eq!(StreamFormat::Sse.content_type(), "text/event-stream");
        assert_eq!(StreamFormat::Ndjson.content_type(), "application/x-ndjson");
        assert_eq!(StreamFormat::Multipart.content_type(), "multipart/x-mixed-replace");
    }

    #[test]
    fn test_validate_buffer_size() {
        let endpoint = HttpStreamingEndpoint {
            default_format: None,
            buffer_size: Some(100),
            enable_compression: None,
        };
        assert!(endpoint.validate(&HashMap::new()).is_ok());

        let endpoint = HttpStreamingEndpoint {
            default_format: None,
            buffer_size: Some(0),
            enable_compression: None,
        };
        assert!(endpoint.validate(&HashMap::new()).is_err());

        let endpoint = HttpStreamingEndpoint {
            default_format: None,
            buffer_size: Some(20000),
            enable_compression: None,
        };
        assert!(endpoint.validate(&HashMap::new()).is_err());
    }

    // ── build_protocol_envelope ──────────────────────────────────────────────

    fn streaming_endpoint() -> HttpStreamingEndpoint {
        HttpStreamingEndpoint {
            default_format: None,
            buffer_size: None,
            enable_compression: None,
        }
    }

    /// A Harmony instance behind dicom_to_http sends its query as a plain HTTP
    /// POST. If the endpoint cannot build an envelope from it, the router
    /// rejects every streaming request with 400.
    #[tokio::test]
    async fn test_build_protocol_envelope_accepts_post_from_dicom_to_http() {
        let mut headers = http::HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("x-dimse-op", "get".parse().unwrap());
        headers.insert("x-query-level", "STUDY".parse().unwrap());
        let body = serde_json::json!({
            "0020000D": { "vr": "UI", "Value": ["1.2.3"] }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let uri: http::Uri = "/stream".parse().unwrap();

        let ctx = crate::adapters::http::common::build_protocol_ctx(
            &Method::POST,
            &uri,
            &headers,
            body_bytes.clone(),
            "/stream",
            "http",
        );

        let envelope = streaming_endpoint()
            .build_protocol_envelope(ctx, &HashMap::new())
            .await
            .expect("an HTTP POST should build a request envelope");

        assert_eq!(envelope.request_details.method, "POST");
        assert_eq!(
            envelope.request_details.headers.get("x-dimse-op").map(String::as_str),
            Some("get")
        );
        assert_eq!(
            envelope.request_details.headers.get("x-query-level").map(String::as_str),
            Some("STUDY")
        );
        assert_eq!(envelope.original_data, body_bytes);
        assert_eq!(envelope.normalized_data, Some(body));
    }

    #[tokio::test]
    async fn test_build_protocol_envelope_rejects_non_http_protocol() {
        let ctx = crate::models::protocol::ProtocolCtx {
            protocol: crate::models::protocol::Protocol::Dimse,
            payload: vec![],
            meta: HashMap::new(),
            attrs: serde_json::json!({}),
            command: None,
        };

        let result = streaming_endpoint()
            .build_protocol_envelope(ctx, &HashMap::new())
            .await;

        assert!(result.is_err(), "only HTTP contexts are supported");
    }
}
