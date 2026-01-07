use crate::config::config::ConfigError;
use crate::models::connection::ConnectionConfig;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

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
                methods: vec![
                    http::Method::GET,
                    http::Method::POST,
                    http::Method::PUT,
                    http::Method::DELETE,
                ],
                description: Some("Handles HTTP requests at exact path".to_string()),
            },
            // Handle subpaths (e.g., /dicom/echo, /api/v1/users)
            RouteConfig {
                path: wildcard_path,
                methods: vec![
                    http::Method::GET,
                    http::Method::POST,
                    http::Method::PUT,
                    http::Method::DELETE,
                ],
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

        // Apply path_prefix from backend options if present
        // This prepends a path to the request URI (e.g., "/api" + "/users" = "/api/users")
        if let Some(path_prefix) = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            let prefix = path_prefix.trim_end_matches('/');
            let uri = if target_details.uri.starts_with('/') {
                &target_details.uri
            } else {
                // Ensure uri has leading slash
                target_details.uri = format!("/{}", target_details.uri);
                &target_details.uri
            };
            target_details.uri = format!("{}{}", prefix, uri);
            tracing::debug!("HTTP backend applied path_prefix '{}' to URI: {}", prefix, target_details.uri);
        }

        tracing::debug!(
            "HTTP backend targeting: {} {}",
            target_details.method,
            target_details
                .full_url()
                .unwrap_or_else(|_| "<invalid-url>".to_string())
        );

        // Store target_details in envelope for future use (Targets model, etc.)
        envelope.target_details = Some(target_details.clone());

        // Check if HTTP/3 (QUIC) is requested
        let use_http3 = connection_config
            .as_ref()
            .is_some_and(|c| c.is_http3());

        let (status, response_headers, body_bytes) = if use_http3 {
            self.make_http3_request(&connection_config.unwrap(), &target_details, &envelope, options)
                .await?
        } else {
            self.make_http_request(&target_details, &envelope, options)
                .await?
        };

        tracing::debug!(
            "HTTP backend response body size: {} bytes",
            body_bytes.len()
        );

        let mut response_envelope = ResponseEnvelope::from_backend(
            envelope.request_details.clone(),
            status,
            response_headers,
            body_bytes,
            None,
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
            method => {
                return Err(Error::from(format!("Unsupported HTTP method: {}", method)));
            }
        };

        // Apply authentication if present in backend options
        request_builder = crate::models::services::backend_auth::apply_backend_authentication(
            request_builder,
            options,
            "HTTP",
        );

        // Add headers from target_details, but drop hop-by-hop and Host headers
        for (key, value) in &target_details.headers {
            let k = key.to_ascii_lowercase();
            if matches!(
                k.as_str(),
                "host"
                    | "connection"
                    | "keep-alive"
                    | "proxy-connection"
                    | "transfer-encoding"
                    | "upgrade"
                    | "content-length"
            ) {
                continue; // let reqwest set correct values from URL/body
            }
            request_builder = request_builder.header(key, value);
        }

        // Add request body if present
        if !envelope.original_data.is_empty() {
            request_builder = request_builder.body(envelope.original_data.clone());
        }

        tracing::debug!("Sending HTTP request to: {}", full_url);

        // Execute the request
        let response = request_builder
            .send()
            .await
            .map_err(|e| Error::from(format!("HTTP request failed: {}", e)))?;

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
            .map_err(|e| Error::from(format!("Failed to read response body: {}", e)))?
            .to_vec();

        Ok((status, response_headers, body_bytes))
    }

    /// Make an HTTP/3 request using QUIC transport
    async fn make_http3_request(
        &self,
        conn: &ConnectionConfig,
        target_details: &crate::models::envelope::envelope::TargetDetails,
        envelope: &RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<(u16, HashMap<String, String>, Vec<u8>), Error> {
        use crate::clients::http3::Http3Client;

        // Create HTTP/3 client - with custom CA if provided
        let client = if let Some(ref ca_path) = conn.ca_cert_path {
            let ca_pem = std::fs::read_to_string(ca_path)
                .map_err(|e| Error::from(format!("Failed to read CA cert at {}: {}", ca_path, e)))?;
            Http3Client::with_ca_cert(&ca_pem)
                .map_err(|e| Error::from(format!("Failed to create HTTP/3 client with CA: {}", e)))?
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

        // Build path with query string from query_params
        let path = if target_details.query_params.is_empty() {
            target_details.uri.clone()
        } else {
            let query_string: String = target_details
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
                .collect::<Vec<_>>()
                .join("&");
            format!("{}?{}", target_details.uri, query_string)
        };

        // Filter headers (drop hop-by-hop headers)
        let mut headers = HashMap::new();
        for (key, value) in &target_details.headers {
            let k = key.to_ascii_lowercase();
            if matches!(
                k.as_str(),
                "host"
                    | "connection"
                    | "keep-alive"
                    | "proxy-connection"
                    | "transfer-encoding"
                    | "upgrade"
                    | "content-length"
            ) {
                continue;
            }
            headers.insert(key.clone(), value.clone());
        }

        tracing::debug!(
            "Sending HTTP/3 request to: {}:{}{}",
            conn.host,
            port,
            path
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
