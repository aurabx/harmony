use crate::config::config::ConfigError;
use crate::models::connection::ConnectionConfig;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::models::services::upstream_headers::{
    upstream_request_headers, InboundAuthorizationPolicy,
};
use crate::router::route_config::{standard_methods, RouteConfig};
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// `ProtocolCtx.meta` keys the HTTP router sets for a request that matched a
/// mesh ingress. They are copied into the request envelope metadata.
const MESH_META_KEYS: [&str; 5] = [
    "mesh_name",
    "mesh_ingress",
    "mesh_pipeline",
    "mesh_provider",
    "mesh_type",
];

#[derive(Debug, Deserialize)]
pub struct HttpEndpoint {}

#[async_trait]
impl ServiceType for HttpEndpoint {
    fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // All path configuration options are optional per the DSL schema:
        // - options.path_prefix (optional)
        // - options.base_url (optional, for backends)
        // - connection.base_path (optional)
        // If none are provided, the service will default to "/" in build_router()
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        let mut path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        if path_prefix.is_empty() {
            if let Some(conn_json) = options.get("connection") {
                if let Ok(conn) = serde_json::from_value::<ConnectionConfig>(conn_json.clone()) {
                    path_prefix = conn.base_path.unwrap_or_else(|| "/".to_string());
                }
            }
        }

        if path_prefix.is_empty() {
            path_prefix = "/".to_string();
        }

        // Ensure clean wildcard path by trimming trailing slashes
        let prefix_trimmed = path_prefix.trim_end_matches('/');
        let wildcard_path = if prefix_trimmed.is_empty() {
            // Root path case
            "/{*wildcard}".to_string()
        } else {
            format!("{}/{{*wildcard}}", prefix_trimmed)
        };

        vec![
            // Handle exact path match
            RouteConfig {
                path: path_prefix.to_string(),
                methods: standard_methods(),
                description: Some("Handles HTTP requests at exact path".to_string()),
            },
            // Handle subpaths (e.g., /dicom/echo, /api/v1/users)
            RouteConfig {
                path: wildcard_path,
                methods: standard_methods(),
                description: Some("Handles HTTP requests with subpaths".to_string()),
            },
        ]
    }

    // noinspection DuplicatedCode
    // Protocol-agnostic builder (HTTP variant)
    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        use crate::models::envelope::envelope::RequestEnvelope;
        use crate::utils::Error;
        use std::collections::HashMap as Map;

        if ctx.protocol != crate::models::protocol::Protocol::Http {
            return Err(Error::from(
                "HttpEndpoint only supports Protocol::Http in build_protocol_envelope",
            ));
        }

        let attrs = ctx
            .attrs
            .as_object()
            .ok_or_else(|| Error::from("invalid attrs for HTTP"))?;
        let headers_map: Map<String, String> = attrs
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let cookies_map: Map<String, String> = attrs
            .get("cookies")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let query_params: Map<String, Vec<String>> = attrs
            .get("query_params")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let vec = v
                            .as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>();
                        (k.clone(), vec)
                    })
                    .collect::<Map<String, Vec<String>>>()
            })
            .unwrap_or_default();
        let cache_status = attrs
            .get("cache_status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut metadata: Map<String, String> = Map::new();
        // pass through HTTP-derived meta (path, path_with_query, full_path) from ctx.meta
        if let Some(path) = ctx.meta.get("path") {
            metadata.insert("path".into(), path.clone());
        }
        if let Some(path_query) = ctx.meta.get("path_with_query") {
            metadata.insert("path_with_query".into(), path_query.clone());
        }
        if let Some(full) = ctx.meta.get("full_path") {
            metadata.insert("full_path".into(), full.clone());
        }
        if let Some(proto) = ctx.meta.get("protocol") {
            metadata.insert("protocol".into(), proto.clone());
        }
        // Peer address recorded by the HTTP adapter's ConnectInfo extractor.
        // Downstream policies (rate limit, ip_allow, ip_deny) key on this.
        if let Some(addr) = ctx.meta.get("remote_addr") {
            metadata.insert("remote_addr".into(), addr.clone());
        }
        if let Some(port) = ctx.meta.get("remote_port") {
            metadata.insert("remote_port".into(), port.clone());
        }
        // Mesh routing context set by the HTTP router. The executor keys
        // ingress mesh_auth injection on `mesh_name`, so dropping these would
        // leave a mesh ingress unauthenticated.
        for key in MESH_META_KEYS {
            if let Some(value) = ctx.meta.get(key) {
                metadata.insert(key.to_string(), value.clone());
            }
        }

        let method = attrs
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let uri = attrs
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // The HTTP method is recorded on the envelope but we do NOT derive a
        // Command from it here. An HTTP method is ambiguous: POST could mean
        // Store, Query (FHIR _search), or Ping (/dicom/echo). Protocol-aware
        // endpoints (DICOMweb, FHIR, DICOM SCP) set the Command in their own
        // endpoint_incoming_request() where they have enough context to
        // disambiguate.
        //
        // When the HTTP endpoint acts as a *backend* (outbound), it DOES read
        // envelope.command to override the HTTP method for protocol bridging
        // (see backend_outgoing_request below).

        // Content-type-aware parsing
        use crate::adapters::http::content_type::*;
        use crate::models::envelope::envelope::{ContentMetadata, ParseStatus};

        let content_type_header = headers_map
            .get("content-type")
            .or_else(|| headers_map.get("Content-Type"))
            .cloned()
            .unwrap_or_else(|| "application/json".to_string());

        let (normalized_data, content_metadata) = if ctx.payload.is_empty() {
            // Empty payload - no parsing needed
            (
                None,
                Some(ContentMetadata {
                    content_type: content_type_header,
                    charset: None,
                    format: "empty".to_string(),
                    parse_status: ParseStatus::NotAttempted,
                    original_size: 0,
                    checksum: None,
                }),
            )
        } else {
            // Parse content-type header
            let ct = parse_content_type(&content_type_header).unwrap_or_else(|_| ContentType {
                media_type: "application/octet-stream".to_string(),
                charset: None,
                boundary: None,
            });

            let original_size = ctx.payload.len();

            // Route to appropriate parser based on content type
            let (parsed_data, format, status, checksum) = match ct.media_type.as_str() {
                // JSON types
                "application/json" | "application/fhir+json" | "application/dicom+json" => {
                    match serde_json::from_slice(&ctx.payload) {
                        Ok(json) => (Some(json), "json".to_string(), ParseStatus::Success, None),
                        Err(e) => {
                            tracing::warn!("Failed to parse JSON: {}", e);
                            (None, "json".to_string(), ParseStatus::Failed, None)
                        }
                    }
                }

                // XML types
                "application/xml" | "text/xml" | "application/soap+xml" => {
                    match parse_xml(&ctx.payload) {
                        Ok(json) => (Some(json), "xml".to_string(), ParseStatus::Success, None),
                        Err(e) => {
                            tracing::warn!("Failed to parse XML: {}", e);
                            (None, "xml".to_string(), ParseStatus::Failed, None)
                        }
                    }
                }

                // CSV
                "text/csv" => match parse_csv(&ctx.payload) {
                    Ok(json) => (Some(json), "csv".to_string(), ParseStatus::Success, None),
                    Err(e) => {
                        tracing::warn!("Failed to parse CSV: {}", e);
                        (None, "csv".to_string(), ParseStatus::Failed, None)
                    }
                },

                // Form URL-encoded
                "application/x-www-form-urlencoded" => match parse_form_urlencoded(&ctx.payload) {
                    Ok(json) => (Some(json), "form".to_string(), ParseStatus::Success, None),
                    Err(e) => {
                        tracing::warn!("Failed to parse form data: {}", e);
                        (None, "form".to_string(), ParseStatus::Failed, None)
                    }
                },

                // Multipart form data (async parsing)
                "multipart/form-data" => match parse_multipart(&ctx.payload, ct.boundary).await {
                    Ok(json) => (
                        Some(json),
                        "multipart".to_string(),
                        ParseStatus::Success,
                        None,
                    ),
                    Err(e) => {
                        tracing::warn!("Failed to parse multipart data: {}", e);
                        (None, "multipart".to_string(), ParseStatus::Failed, None)
                    }
                },

                // Binary content
                media_type if is_binary_content(media_type) => {
                    let checksum = calculate_checksum(&ctx.payload);
                    let metadata = create_binary_metadata(media_type, &ctx.payload);
                    (
                        Some(metadata),
                        "binary".to_string(),
                        ParseStatus::Success,
                        Some(checksum),
                    )
                }

                // Unknown/unsupported - try JSON as fallback
                _ => match serde_json::from_slice(&ctx.payload) {
                    Ok(json) => (Some(json), "json".to_string(), ParseStatus::Success, None),
                    Err(_) => (None, "unknown".to_string(), ParseStatus::Unsupported, None),
                },
            };

            let metadata = ContentMetadata {
                content_type: content_type_header,
                charset: ct.charset,
                format,
                parse_status: status,
                original_size,
                checksum,
            };

            (parsed_data, Some(metadata))
        };

        RequestEnvelope::builder()
            .method(method)
            .uri(uri)
            .headers(headers_map)
            .cookies(cookies_map)
            .query_params(query_params)
            .cache_status(cache_status)
            .metadata(metadata)
            .content_metadata(content_metadata)
            .command(None)
            .target_details(None)
            .original_data(ctx.payload)
            .normalized_data(normalized_data)
            .normalized_snapshot(None)
            .build()
    }
}

#[async_trait]
impl ServiceHandler<Value> for HttpEndpoint {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Populate normalized data with real request context

        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        use crate::models::envelope::envelope::TargetDetails;

        // Parse connection config if available
        let connection_config = options
            .get("connection")
            .and_then(|v| serde_json::from_value::<ConnectionConfig>(v.clone()).ok());

        // Extract base_url from backend options or connection config
        let base_url = if let Some(ref conn) = connection_config {
            conn.to_base_url()
        } else {
            options
                .get("base_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };

        if base_url.is_empty() {
            return Err(Error::from(
                "HTTP backend requires 'base_url' in options or valid 'connection' config",
            ));
        }

        // Use target_details from executor (already populated from request_details + backend config)
        // Fill in base_url if not already set by executor
        let mut target_details = if let Some(mut target) = envelope.target_details.take() {
            if target.base_url.is_empty() {
                target.base_url = base_url.to_string();
            }
            target
        } else {
            // Fallback: create from request_details (shouldn't normally happen)
            let path = crate::models::services::path_utils::extract_path(&envelope);
            let mut target = TargetDetails::from_request_details(
                base_url.to_string(),
                &envelope.request_details,
            );
            target.uri = path;
            target
        };

        // If an explicit Command is set on the envelope, use it to override the HTTP method
        // This enables protocol-bridging scenarios (e.g., DICOM -> HTTP)
        if let Some(ref cmd) = envelope.command {
            use crate::models::envelope::command::Operation;
            let overridden_method = match cmd.operation {
                Operation::Query | Operation::Retrieve => "GET",
                Operation::Store => "POST",
                Operation::Update => "PUT",
                Operation::Patch => "PATCH",
                Operation::Delete => "DELETE",
                Operation::Ping => "HEAD",
                _ => target_details.method.as_str(),
            };
            if overridden_method != target_details.method {
                tracing::debug!(
                    "HTTP backend: Command({}) overriding method {} -> {}",
                    cmd.operation,
                    target_details.method,
                    overridden_method
                );
                target_details.method = overridden_method.to_string();
            }
        }

        let path_prefix = options.get("path_prefix").and_then(|v| v.as_str());
        if has_non_http_scheme(&target_details.uri) {
            // The rest of the URI can carry identifiers, so only the scheme
            // is logged. has_non_http_scheme guarantees "://" is present.
            let scheme = target_details.uri.split("://").next().unwrap_or_default();
            tracing::debug!(
                "HTTP backend ignoring non-HTTP request URI with scheme '{}' as backend path",
                scheme
            );
        }
        target_details.uri = backend_request_path(&target_details.uri, path_prefix);

        tracing::debug!(
            "HTTP backend targeting: {} {}",
            target_details.method,
            target_details
                .full_url()
                .map(|url| crate::logging::without_query(&url).to_string())
                .unwrap_or_else(|_| "<invalid-url>".to_string())
        );

        // Store target_details in envelope for future use (Targets model, etc.)
        envelope.target_details = Some(target_details.clone());

        // Check if HTTP/3 (QUIC) is requested
        let use_http3 = connection_config.as_ref().is_some_and(|c| c.is_http3());

        let (status, response_headers, body_bytes) = if use_http3 {
            self.make_http3_request(
                &connection_config.unwrap(),
                &target_details,
                &envelope,
                options,
            )
            .await?
        } else {
            self.make_http_request(&target_details, &envelope, options)
                .await?
        };

        tracing::debug!(
            "HTTP backend response body size: {} bytes",
            body_bytes.len()
        );

        let mut response_envelope = ResponseEnvelope::from_backend_with_command(
            envelope.request_details.clone(),
            status,
            response_headers,
            body_bytes,
            None,
            envelope.command.clone(),
        );

        // Try to parse response as JSON if content-type indicates JSON
        if let Some(content_type) = response_envelope
            .response_details
            .headers
            .get("content-type")
        {
            if content_type.contains("application/json") {
                if let Ok(json_value) =
                    serde_json::from_slice::<serde_json::Value>(&response_envelope.original_data)
                {
                    response_envelope.normalized_data = Some(json_value);
                }
            }
        }

        Ok(response_envelope)
    }

    /// Protocol-aware response post-processing
    ///
    /// For HTTP service, this adds protocol metadata to response headers
    /// to help with debugging and observability.
    async fn endpoint_outgoing_protocol(
        &self,
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Add protocol information to response metadata for observability
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));

        // For HTTP protocol, optionally add X-Protocol header for debugging
        if ctx.protocol == crate::models::protocol::Protocol::Http {
            envelope
                .response_details
                .headers
                .entry("x-harmony-protocol".to_string())
                .or_insert_with(|| "http".to_string());
        }

        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Build response from ResponseEnvelope
        let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        // Add headers from response_details, but skip hop-by-hop headers
        // including content-length (let hyper set it from actual body)
        for (k, v) in &envelope.response_details.headers {
            let key_lower = k.to_ascii_lowercase();
            if matches!(
                key_lower.as_str(),
                "content-length" | "transfer-encoding" | "connection" | "keep-alive"
            ) {
                continue;
            }
            builder = builder.header(k.as_str(), v.as_str());
        }

        // Use original_data if available, otherwise serialize normalized_data
        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else if let Some(normalized) = envelope.normalized_data {
            let body_bytes = serde_json::to_vec(&normalized)
                .map_err(|_| Error::from("Failed to serialize HTTP response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct HTTP response"))
    }
}

/// HTTP/1.1/2 and HTTP/3 request helpers for HttpEndpoint
impl HttpEndpoint {
    /// Make an HTTP/1.1 or HTTP/2 request using reqwest
    async fn make_http_request(
        &self,
        target_details: &crate::models::envelope::envelope::TargetDetails,
        envelope: &RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<(u16, HashMap<String, String>, Vec<u8>), Error> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::from(format!("Failed to create HTTP client: {}", e)))?;

        let full_url = target_details.full_url()?;

        // Build the request
        let mut request_builder = match target_details.method.as_str() {
            "GET" => client.get(&full_url),
            "POST" => client.post(&full_url),
            "PUT" => client.put(&full_url),
            "DELETE" => client.delete(&full_url),
            "PATCH" => client.patch(&full_url),
            "HEAD" => client.head(&full_url),
            "OPTIONS" => client.request(reqwest::Method::OPTIONS, &full_url),
            method => {
                return Err(Error::from(format!("Unsupported HTTP method: {}", method)));
            }
        };

        let mut headers = upstream_request_headers(
            &target_details.headers,
            &envelope.request_details.headers,
            InboundAuthorizationPolicy::from_backend_options(options)?,
            "HTTP",
        );
        // RequestBuilder::header appends, so the target's credential must go
        // into the map, where it replaces a same-named header in any letter case.
        crate::models::services::backend_auth::apply_backend_authentication_to_headers(
            &mut headers,
            options,
            "HTTP",
        );
        for (key, value) in headers {
            request_builder = request_builder.header(key, value);
        }

        // Add request body if present
        if !envelope.original_data.is_empty() {
            request_builder = request_builder.body(envelope.original_data.clone());
        }

        tracing::debug!(
            "Sending HTTP request to: {}",
            crate::logging::without_query(&full_url)
        );

        // Execute the request
        let response = request_builder
            .send()
            .await
            // reqwest appends the full URL, query included, to its error
            // text, and the router logs that text at ERROR.
            .map_err(|e| Error::from(format!("HTTP request failed: {}", e.without_url())))?;

        let status = response.status().as_u16();
        tracing::debug!("HTTP backend response status: {}", status);

        // Extract response headers
        let mut response_headers = HashMap::new();
        for (key, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                response_headers.insert(key.to_string(), value_str.to_string());
            }
        }

        // Get response body
        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| Error::from(format!("Failed to read response body: {}", e.without_url())))?
            .to_vec();

        Ok((status, response_headers, body_bytes))
    }

    /// Make an HTTP/3 request using QUIC transport
    async fn make_http3_request(
        &self,
        conn: &ConnectionConfig,
        target_details: &crate::models::envelope::envelope::TargetDetails,
        envelope: &RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<(u16, HashMap<String, String>, Vec<u8>), Error> {
        use crate::clients::http3::Http3Client;

        // Create HTTP/3 client - with custom CA if provided
        let client = if let Some(ref ca_path) = conn.ca_cert_path {
            let ca_pem = std::fs::read_to_string(ca_path).map_err(|e| {
                Error::from(format!("Failed to read CA cert at {}: {}", ca_path, e))
            })?;
            Http3Client::with_ca_cert(&ca_pem).map_err(|e| {
                Error::from(format!("Failed to create HTTP/3 client with CA: {}", e))
            })?
        } else {
            Http3Client::new()
                .map_err(|e| Error::from(format!("Failed to create HTTP/3 client: {}", e)))?
        };

        let port = conn.port.unwrap_or(443);

        // Parse method
        let method = target_details
            .method
            .parse::<http::Method>()
            .map_err(|_| Error::from(format!("Invalid HTTP method: {}", target_details.method)))?;

        // The request path includes the connection's base_path, as the reqwest path does via full_url().
        let path = crate::models::services::path_utils::http3_request_path(
            conn.base_path.as_deref(),
            &target_details.uri,
            &target_details.query_params,
        );

        let mut headers = upstream_request_headers(
            &target_details.headers,
            &envelope.request_details.headers,
            InboundAuthorizationPolicy::from_backend_options(options)?,
            "HTTP/3",
        );
        // Target authentication: the inbound Authorization was already dropped above, so
        // the target's credential is the only one sent.
        crate::models::services::backend_auth::apply_backend_authentication_to_headers(
            &mut headers,
            options,
            "HTTP/3",
        );

        tracing::debug!(
            "Sending HTTP/3 request to: {}:{}{}",
            conn.host,
            port,
            crate::logging::without_query(&path)
        );

        let response = client
            .request(
                method,
                &conn.host,
                port,
                &path,
                &headers,
                envelope.original_data.clone(),
            )
            .await
            .map_err(|e| Error::from(format!("HTTP/3 request failed: {}", e)))?;

        let status = response.status.as_u16();
        tracing::debug!("HTTP/3 backend response status: {}", status);

        Ok((status, response.headers, response.body))
    }
}

/// Return true when `uri` is an absolute URI whose scheme is not HTTP(S).
///
/// Non-HTTP endpoints put protocol URIs such as `dimse://scp/store` into the
/// request URI. Those URIs identify the inbound operation, not a path on the
/// HTTP backend, so they must never be joined onto the backend base URL.
fn has_non_http_scheme(uri: &str) -> bool {
    let Some((scheme, _)) = uri.split_once("://") else {
        return false;
    };
    // RFC 3986 scheme syntax. A value such as `/proxy/http://x` has a '/'
    // before "://", so it is a path and not a scheme URI.
    let mut chars = scheme.chars();
    let is_scheme = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    is_scheme && !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https")
}

/// Build the path sent to the HTTP backend from the executor's target URI
/// and the backend's optional `path_prefix`.
///
/// HTTP request paths keep their existing behaviour: the prefix (trailing
/// slashes trimmed) is prepended and a leading slash is ensured. A non-HTTP
/// request URI is dropped, so the result is the prefix alone. With no prefix
/// that is an empty path, which makes `full_url()` return the base URL
/// (target root or `base_path`) unchanged.
fn backend_request_path(uri: &str, path_prefix: Option<&str>) -> String {
    let prefix = path_prefix
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim_end_matches('/'));

    if has_non_http_scheme(uri) {
        return prefix.unwrap_or_default().to_string();
    }

    match prefix {
        Some(prefix) if uri.starts_with('/') => format!("{}{}", prefix, uri),
        Some(prefix) => format!("{}/{}", prefix, uri),
        None => uri.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::TargetDetails;

    #[test]
    fn test_backend_request_path_http_path_without_prefix_is_unchanged() {
        assert_eq!(backend_request_path("/api/users", None), "/api/users");
    }

    #[test]
    fn test_backend_request_path_http_path_gets_prefix() {
        assert_eq!(backend_request_path("/users", Some("/api/")), "/api/users");
        assert_eq!(backend_request_path("users", Some("/api")), "/api/users");
    }

    #[test]
    fn test_backend_request_path_blank_prefix_is_ignored() {
        assert_eq!(backend_request_path("/users", Some("  ")), "/users");
    }

    #[test]
    fn test_backend_request_path_absolute_http_url_is_not_dropped() {
        assert!(!has_non_http_scheme("http://example.com/a"));
        assert!(!has_non_http_scheme("HTTPS://example.com/a"));
    }

    #[test]
    fn test_backend_request_path_path_containing_scheme_text_is_kept() {
        assert!(!has_non_http_scheme("/proxy/http://example.com"));
        assert_eq!(
            backend_request_path("/proxy/dimse://x", Some("/api")),
            "/api/proxy/dimse://x"
        );
    }

    #[test]
    fn test_backend_request_path_drops_dimse_uri() {
        assert!(has_non_http_scheme("dimse://scp/store"));
        assert_eq!(backend_request_path("dimse://scp/store", None), "");
        assert_eq!(
            backend_request_path("dimse://scp/store", Some("/ingest/")),
            "/ingest"
        );
    }

    #[test]
    fn test_backend_request_path_drops_other_non_http_schemes() {
        assert_eq!(backend_request_path("hl7://mllp/store", Some("/in")), "/in");
        assert_eq!(backend_request_path("mllp+tls://x", None), "");
    }

    #[test]
    fn test_full_url_for_dropped_dimse_uri_is_target_root() {
        let target = TargetDetails {
            base_url: "http://127.0.0.1:8080".to_string(),
            uri: backend_request_path("dimse://scp/store", None),
            ..TargetDetails::default()
        };
        let url = target.full_url().expect("full_url");
        assert_eq!(url, "http://127.0.0.1:8080");
        let parsed = reqwest::Url::parse(&url).expect("valid URL");
        assert_eq!(parsed.path(), "/");
    }

    /// A search request with a patient name and birthdate, sent to a
    /// plain-HTTP backend on `port`.
    fn search_target(port: u16, marker: &str) -> TargetDetails {
        let mut query_params = HashMap::new();
        query_params.insert("name".to_string(), vec![marker.to_string()]);
        query_params.insert("birthdate".to_string(), vec!["1970-01-01".to_string()]);
        TargetDetails {
            base_url: format!("http://127.0.0.1:{}", port),
            method: "GET".to_string(),
            uri: "/Patient".to_string(),
            query_params,
            ..TargetDetails::default()
        }
    }

    fn empty_envelope() -> RequestEnvelope<Vec<u8>> {
        RequestEnvelope::new(Default::default(), vec![])
    }

    /// reqwest appends the full URL, query included, to its error text. The
    /// router logs that text at ERROR, the default level.
    #[tokio::test]
    async fn send_error_does_not_contain_search_values() {
        let marker = "DOE-PHI-MARKER";
        let err = HttpEndpoint {}
            .make_http_request(
                &search_target(1, marker),
                &empty_envelope(),
                &HashMap::new(),
            )
            .await
            .err()
            .expect("port 1 must refuse the connection");

        let text = format!("{} {:?}", err, err);
        assert!(text.contains("HTTP request failed"), "error: {text}");
        assert!(!text.contains(marker), "search values leaked: {text}");
        assert!(!text.contains("birthdate"), "query string leaked: {text}");
    }

    #[tokio::test]
    async fn debug_log_does_not_contain_query_string() {
        let (log, _guard) = crate::logging::test_capture::capture_debug();
        let marker = "DOE-PHI-MARKER";
        let _ = HttpEndpoint {}
            .make_http_request(
                &search_target(1, marker),
                &empty_envelope(),
                &HashMap::new(),
            )
            .await;

        let text = log.contents();
        assert!(
            text.contains("Sending HTTP request to: http://127.0.0.1:1/Patient"),
            "log: {text}"
        );
        assert!(!text.contains(marker), "search values leaked: {text}");
        assert!(!text.contains("birthdate"), "query string leaked: {text}");
    }

    /// The executor-facing entry point logs the target before sending.
    #[tokio::test]
    async fn targeting_log_does_not_contain_query_string() {
        let (log, _guard) = crate::logging::test_capture::capture_debug();
        let marker = "DOE-PHI-MARKER";
        let mut envelope = empty_envelope();
        envelope.target_details = Some(search_target(1, marker));
        let mut options = HashMap::new();
        options.insert(
            "base_url".to_string(),
            Value::String("http://127.0.0.1:1".to_string()),
        );
        let _ = HttpEndpoint {}
            .backend_outgoing_request(envelope, &options)
            .await;

        let text = log.contents();
        assert!(
            text.contains("HTTP backend targeting: GET http://127.0.0.1:1/Patient"),
            "log: {text}"
        );
        assert!(!text.contains(marker), "search values leaked: {text}");
        assert!(!text.contains("birthdate"), "query string leaked: {text}");
    }

    /// A DIMSE request URI can carry identifiers after the scheme.
    #[tokio::test]
    async fn non_http_uri_log_shows_only_the_scheme() {
        let (log, _guard) = crate::logging::test_capture::capture_debug();
        let marker = "DOE-PHI-MARKER";
        let mut envelope = empty_envelope();
        envelope.target_details = Some(TargetDetails {
            base_url: "http://127.0.0.1:1".to_string(),
            method: "POST".to_string(),
            uri: format!("dimse://scp/{}", marker),
            ..TargetDetails::default()
        });
        let mut options = HashMap::new();
        options.insert(
            "base_url".to_string(),
            Value::String("http://127.0.0.1:1".to_string()),
        );
        let _ = HttpEndpoint {}
            .backend_outgoing_request(envelope, &options)
            .await;

        let text = log.contents();
        assert!(
            text.contains("HTTP backend ignoring non-HTTP request URI with scheme 'dimse'"),
            "log: {text}"
        );
        assert!(!text.contains(marker), "request URI leaked: {text}");
    }

    /// A backend that promises more body than it sends makes reading the
    /// body fail after the status line has arrived. reqwest 0.12 leaves the
    /// URL out of this error today, so this guards against it adding one.
    #[tokio::test]
    async fn body_read_error_does_not_contain_search_values() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort")
                .await;
        });

        let marker = "DOE-PHI-MARKER";
        let err = HttpEndpoint {}
            .make_http_request(
                &search_target(port, marker),
                &empty_envelope(),
                &HashMap::new(),
            )
            .await
            .err()
            .expect("a truncated body must fail");

        let text = format!("{} {:?}", err, err);
        assert!(
            text.contains("Failed to read response body"),
            "error: {text}"
        );
        assert!(!text.contains(marker), "search values leaked: {text}");
        assert!(!text.contains("birthdate"), "query string leaked: {text}");
    }
}
