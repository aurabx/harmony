use crate::config::config::ConfigError;
use crate::models::connection::ConnectionConfig;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope, TargetDetails};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::models::services::upstream_headers::{
    upstream_request_headers, InboundAuthorizationPolicy,
};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::response::Response;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// JMIX Backend Service
///
/// Forwards JMIX requests (GET/POST) to upstream JMIX servers.
/// Supports:
/// - POST /api/jmix (upload envelope)
/// - GET /api/jmix/{id} (retrieve envelope by ID)
/// - GET /api/jmix/{id}/manifest (retrieve manifest)
/// - GET /api/jmix?studyInstanceUid=... (query by study UID)
#[derive(Debug, Deserialize)]
pub struct JmixBackend {}

#[async_trait]
impl ServiceType for JmixBackend {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // Validate that connection config is present
        let has_connection = options.get("connection").is_some();
        let has_base_url = options
            .get("base_url")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);

        if !has_connection && !has_base_url {
            return Err(ConfigError::InvalidBackend {
                name: "jmix_backend".to_string(),
                reason: "JMIX backend requires 'connection' config or 'base_url' option"
                    .to_string(),
            });
        }

        Ok(())
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Backends don't define routes - they only handle outbound requests
        vec![]
    }

    async fn build_protocol_envelope(
        &self,
        _ctx: crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        Err(Error::from(
            "JmixBackend does not support build_protocol_envelope (backend-only service)",
        ))
    }
}

#[async_trait]
impl ServiceHandler<Value> for JmixBackend {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Backends don't process incoming requests from clients
        // This path is only used when backend is mistakenly configured as an endpoint
        Ok(envelope)
    }

    async fn backend_outgoing_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
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
                "JMIX backend requires 'base_url' in options or valid 'connection' config",
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

        // Normalise the target URI before path_prefix is applied.
        //
        // When a DIMSE endpoint feeds this backend (for example via the
        // dimse_to_jmix middleware) the incoming envelope URI is a
        // protocol URI like `dimse://scp/store`, not an HTTP path. The
        // executor propagates that URI into target_details.uri, and any
        // downstream path_prefix concatenation would produce an invalid
        // URL such as `/api/dimse://scp/store`. Rewrite recognised
        // protocol URIs to their JMIX HTTP equivalent here so path_prefix
        // and full_url() both see a well-formed path.
        target_details.uri = map_backend_uri(&target_details.uri)?;

        // Apply path_prefix from backend options if present
        // This prepends a path to the request URI (e.g., "/api/jmix" + "/123" = "/api/jmix/123")
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
            tracing::debug!(
                "JMIX backend applied path_prefix '{}' to URI: {}",
                prefix,
                target_details.uri
            );
        }

        tracing::debug!(
            "JMIX backend targeting: {} {}",
            target_details.method,
            target_details
                .full_url()
                .map(|url| crate::logging::without_query(&url).to_string())
                .unwrap_or_else(|_| "<invalid-url>".to_string())
        );

        // If an explicit Command is set on the envelope, use it to override the HTTP method.
        // The JMIX endpoint only produces Query, Retrieve, and Store operations,
        // so we only map those here.
        if let Some(ref cmd) = envelope.command {
            use crate::models::envelope::command::Operation;
            let overridden_method = match cmd.operation {
                Operation::Query | Operation::Retrieve => "GET",
                Operation::Store => "POST",
                _ => target_details.method.as_str(),
            };
            if overridden_method != target_details.method {
                tracing::debug!(
                    "JMIX backend: Command({}) overriding method {} -> {}",
                    cmd.operation,
                    target_details.method,
                    overridden_method
                );
                target_details.method = overridden_method.to_string();
            }
        }

        // Store target_details in envelope for future use
        envelope.target_details = Some(target_details.clone());

        // Make the HTTP request to upstream JMIX server
        let (status, response_headers, body_bytes) = self
            .make_jmix_request(&target_details, &envelope, options)
            .await?;

        tracing::debug!(
            "JMIX backend response status: {}, body size: {} bytes",
            status,
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

    async fn endpoint_outgoing_protocol(
        &self,
        _envelope: &mut ResponseEnvelope<Vec<u8>>,
        _ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Backends don't process outgoing protocol responses
        Ok(())
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
        // Backends don't construct final HTTP responses
        // This is handled by the endpoint service
        // Return a basic response as fallback
        use axum::body::Body;

        let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        for (k, v) in &envelope.response_details.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct JMIX backend response"))
    }
}

/// Normalise a target URI for the JMIX HTTP backend.
///
/// The executor sets `target_details.uri` from the incoming request's path.
/// For HTTP-facing endpoints that is already an HTTP path. For endpoints
/// whose protocol is not HTTP (DIMSE C-STORE, for example) the URI carries
/// a protocol scheme and would produce an invalid HTTP URL if concatenated
/// with `path_prefix`. This helper rewrites the recognised protocol URIs
/// to their JMIX HTTP equivalents and rejects unknown ones.
fn map_backend_uri(uri: &str) -> Result<String, Error> {
    // HTTP-shaped paths (or an empty URI) pass through unchanged.
    if uri.is_empty() || uri.starts_with('/') {
        return Ok(uri.to_string());
    }

    // Absolute HTTP(S) URLs are accepted verbatim; downstream `full_url()`
    // already knows how to handle them.
    let lowered = uri.to_ascii_lowercase();
    if lowered.starts_with("http://") || lowered.starts_with("https://") {
        return Ok(uri.to_string());
    }

    // Recognised non-HTTP schemes: map to the JMIX resource that carries
    // the same intent.
    if lowered.starts_with("dimse://scp/store") {
        return Ok("/jmix".to_string());
    }

    // Any other scheme is a bug in the pipeline wiring: concatenating a
    // path_prefix onto it would produce a garbage URL. Refuse loudly so
    // the failure is visible in logs instead of arriving as a 404 or
    // connection error from the upstream.
    if let Some(idx) = lowered.find("://") {
        let scheme = &lowered[..idx];
        return Err(Error::from(format!(
            "JMIX backend received URI with unsupported scheme '{}': {}. \
             Add a mapping in map_backend_uri or fix the pipeline wiring.",
            scheme, uri
        )));
    }

    // Not a scheme URI and not a path. Treat as a relative path — matches
    // the historical behaviour for callers that pass bare identifiers.
    Ok(format!("/{}", uri))
}

/// HTTP request helper for JmixBackend
impl JmixBackend {
    /// Make an HTTP request to upstream JMIX server
    async fn make_jmix_request(
        &self,
        target_details: &TargetDetails,
        envelope: &RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<(u16, HashMap<String, String>, Vec<u8>), Error> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::from(format!("Failed to create HTTP client: {}", e)))?;

        let full_url = target_details.full_url()?;

        // Build the request – JMIX only uses GET (query/retrieve) and POST (store)
        let mut request_builder = match target_details.method.as_str() {
            "GET" | "HEAD" => client.get(&full_url),
            "POST" => client.post(&full_url),
            method => {
                return Err(Error::from(format!(
                    "Unsupported HTTP method for JMIX backend: {}",
                    method
                )));
            }
        };

        let mut headers = upstream_request_headers(
            &target_details.headers,
            &envelope.request_details.headers,
            InboundAuthorizationPolicy::from_backend_options(options)?,
            "JMIX",
        );
        // RequestBuilder::header appends, so the target's credential must go
        // into the map, where it replaces a same-named header in any letter case.
        crate::models::services::backend_auth::apply_backend_authentication_to_headers(
            &mut headers,
            options,
            "JMIX",
        );
        for (key, value) in headers {
            request_builder = request_builder.header(key, value);
        }

        // Add request body if present (for POST with ZIP uploads)
        if !envelope.original_data.is_empty() {
            request_builder = request_builder.body(envelope.original_data.clone());
        }

        tracing::debug!(
            "Sending JMIX request to upstream: {}",
            crate::logging::without_query(&full_url)
        );

        // Execute the request
        let response = request_builder
            .send()
            .await
            // reqwest appends the full URL, query included, to its error
            // text, and the router logs that text at ERROR.
            .map_err(|e| {
                Error::from(format!(
                    "JMIX request to upstream failed: {}",
                    e.without_url()
                ))
            })?;

        let status = response.status().as_u16();
        tracing::debug!("JMIX upstream response status: {}", status);

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
            .map_err(|e| {
                Error::from(format!(
                    "Failed to read JMIX response body: {}",
                    e.without_url()
                ))
            })?
            .to_vec();

        Ok((status, response_headers, body_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_with_connection() {
        let backend = JmixBackend {};
        let mut options = HashMap::new();
        let connection = ConnectionConfig {
            host: "upstream.example.com".to_string(),
            port: Some(443),
            protocol: Some("https".to_string()),
            ..Default::default()
        };
        options.insert(
            "connection".to_string(),
            serde_json::to_value(connection).unwrap(),
        );
        assert!(backend.validate(&options).is_ok());
    }

    #[test]
    fn test_validate_with_base_url() {
        let backend = JmixBackend {};
        let mut options = HashMap::new();
        options.insert(
            "base_url".to_string(),
            serde_json::json!("https://upstream.example.com"),
        );
        assert!(backend.validate(&options).is_ok());
    }

    #[test]
    fn test_validate_missing_both() {
        let backend = JmixBackend {};
        let options = HashMap::new();
        assert!(backend.validate(&options).is_err());
    }

    #[test]
    fn test_build_router_empty() {
        let backend = JmixBackend {};
        let options = HashMap::new();
        let routes = backend.build_router(&options);
        assert!(routes.is_empty(), "Backends should not define routes");
    }

    // ── map_backend_uri ─────────────────────────────────────────────────

    #[test]
    fn test_map_backend_uri_passes_through_http_path() {
        // A plain HTTP path is what the executor produces for HTTP endpoints;
        // it must survive intact so path_prefix concatenation still works.
        assert_eq!(
            map_backend_uri("/api/jmix/1234").expect("path passes through"),
            "/api/jmix/1234"
        );
    }

    #[test]
    fn test_map_backend_uri_passes_through_absolute_http_url() {
        assert_eq!(
            map_backend_uri("https://example.com/api/jmix").expect("http url passes"),
            "https://example.com/api/jmix"
        );
    }

    #[test]
    fn test_map_backend_uri_rewrites_dimse_cstore_to_jmix() {
        // The DIMSE C-STORE envelope URI is not an HTTP path. When a DIMSE
        // pipeline forwards to a JMIX backend the aggregation middleware
        // has already put the ZIP bytes into original_data, but the URI
        // is still the DIMSE placeholder. Rewrite it to /jmix so the
        // subsequent path_prefix ("/api") produces "/api/jmix".
        assert_eq!(
            map_backend_uri("dimse://scp/store").expect("dimse rewrites"),
            "/jmix"
        );
    }

    #[test]
    fn test_map_backend_uri_rejects_unknown_scheme() {
        let err = map_backend_uri("hl7://mllp/store")
            .expect_err("unknown scheme must not be concatenated with path_prefix");
        let msg = format!("{}", err);
        assert!(
            msg.contains("hl7"),
            "error should name the offending scheme: {}",
            msg
        );
    }

    #[test]
    fn test_map_backend_uri_bare_identifier_gets_leading_slash() {
        assert_eq!(
            map_backend_uri("abc-123").expect("bare id becomes path"),
            "/abc-123"
        );
    }

    #[test]
    fn test_map_backend_uri_empty_is_root() {
        assert_eq!(map_backend_uri("").expect("empty ok"), "");
    }

    /// A JMIX query with a patient name and birthdate, sent to a plain-HTTP
    /// upstream on `port`.
    fn search_target(port: u16, marker: &str) -> TargetDetails {
        let mut query_params = HashMap::new();
        query_params.insert("patient_name".to_string(), vec![marker.to_string()]);
        query_params.insert("birthdate".to_string(), vec!["1970-01-01".to_string()]);
        TargetDetails {
            base_url: format!("http://127.0.0.1:{}", port),
            method: "GET".to_string(),
            uri: "/api/jmix".to_string(),
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
        let err = JmixBackend {}
            .make_jmix_request(
                &search_target(1, marker),
                &empty_envelope(),
                &HashMap::new(),
            )
            .await
            .err()
            .expect("port 1 must refuse the connection");

        let text = format!("{} {:?}", err, err);
        assert!(
            text.contains("JMIX request to upstream failed"),
            "error: {text}"
        );
        assert!(!text.contains(marker), "search values leaked: {text}");
        assert!(!text.contains("birthdate"), "query string leaked: {text}");
    }

    #[tokio::test]
    async fn debug_log_does_not_contain_query_string() {
        let (log, _guard) = crate::logging::test_capture::capture_debug();
        let marker = "DOE-PHI-MARKER";
        let _ = JmixBackend {}
            .make_jmix_request(
                &search_target(1, marker),
                &empty_envelope(),
                &HashMap::new(),
            )
            .await;

        let text = log.contents();
        assert!(
            text.contains("Sending JMIX request to upstream: http://127.0.0.1:1/api/jmix"),
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
        let _ = JmixBackend {}
            .backend_outgoing_request(envelope, &options)
            .await;

        let text = log.contents();
        assert!(
            text.contains("JMIX backend targeting: GET http://127.0.0.1:1/api/jmix"),
            "log: {text}"
        );
        assert!(!text.contains(marker), "search values leaked: {text}");
        assert!(!text.contains("birthdate"), "query string leaked: {text}");
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
        let err = JmixBackend {}
            .make_jmix_request(
                &search_target(port, marker),
                &empty_envelope(),
                &HashMap::new(),
            )
            .await
            .err()
            .expect("a truncated body must fail");

        let text = format!("{} {:?}", err, err);
        assert!(
            text.contains("Failed to read JMIX response body"),
            "error: {text}"
        );
        assert!(!text.contains(marker), "search values leaked: {text}");
        assert!(!text.contains("birthdate"), "query string leaked: {text}");
    }
}
