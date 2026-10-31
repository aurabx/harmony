use crate::config::config::ConfigError;
use crate::models::connection::ConnectionConfig;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::models::services::upstream_headers::{
    upstream_request_headers, InboundAuthorizationPolicy,
};
use crate::router::route_config::RouteConfig;
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct FhirEndpoint {}

#[async_trait]
impl ServiceType for FhirEndpoint {
    fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        // All path configuration options are optional per the DSL schema:
        // - options.path_prefix (optional)
        // - connection.base_path (optional)
        // If none are provided, the service will default to "/fhir" in build_router()
        Ok(())
    }

    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
        // Get the 'path_prefix' from options or default to "/fhir"
        let mut path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        if path_prefix.is_empty() {
            if let Some(conn_json) = options.get("connection") {
                if let Ok(conn) = serde_json::from_value::<ConnectionConfig>(conn_json.clone()) {
                    path_prefix = conn.base_path.unwrap_or_default();
                }
            }
        }

        if path_prefix.is_empty() {
            path_prefix = "/fhir".to_string();
        }

        // Return route configurations for GET/POST/PUT/DELETE methods
        vec![RouteConfig {
            path: format!("{}/{{*wildcard}}", path_prefix), // Use {*wildcard} syntax
            methods: vec![
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::HEAD,
                Method::OPTIONS,
            ],
            description: Some("Handles FHIR RESTful API requests".to_string()),
        }]
    }

    async fn build_protocol_envelope(
        &self,
        ctx: crate::models::protocol::ProtocolCtx,
        options: &HashMap<String, Value>,
    ) -> Result<crate::models::envelope::envelope::RequestEnvelope<Vec<u8>>, crate::utils::Error>
    {
        // Delegate to HttpEndpoint for HTTP variant
        let http = crate::models::services::types::http::HttpEndpoint {};
        http.build_protocol_envelope(ctx, options).await
    }
}

#[async_trait]
impl ServiceHandler<Value> for FhirEndpoint {
    type ReqBody = Value;

    // Process the incoming request and transform it into an Envelope
    async fn endpoint_incoming_request(
        &self,
        mut envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        // Refine the Command with FHIR-specific operation semantics
        use crate::models::envelope::command::{Command, Operation};

        let method = envelope.request_details.method.to_uppercase();
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();
        let parts: Vec<&str> = subpath.split('/').filter(|s| !s.is_empty()).collect();

        let operation = match method.as_str() {
            "GET" => match parts.as_slice() {
                // GET /metadata -> Ping (capability statement)
                ["metadata"] => Some(Operation::Ping),
                // GET / -> whole-system search
                [] => Some(Operation::Query),
                // GET /_history -> whole-system history
                ["_history"] => Some(Operation::Query),
                // GET /ResourceType/_history -> type-level history
                [_resource_type, "_history"] => Some(Operation::Query),
                // GET /ResourceType/_search -> search (unusual but valid)
                [_resource_type, "_search"] => Some(Operation::Query),
                // GET /ResourceType/$operation -> type-level operation
                [_resource_type, op] if op.starts_with('$') => Some(Operation::Retrieve),
                // GET /ResourceType -> search (Query)
                [_resource_type] => Some(Operation::Query),
                // GET /ResourceType/id -> read (Retrieve)
                [_resource_type, _id] => Some(Operation::Retrieve),
                // GET /ResourceType/id/_history -> instance history (Query)
                [_resource_type, _id, "_history"] => Some(Operation::Query),
                // GET /ResourceType/id/_history/vid -> vread (Retrieve)
                [_resource_type, _id, "_history", _vid] => Some(Operation::Retrieve),
                // GET /ResourceType/id/$operation -> instance-level operation
                [_resource_type, _id, op] if op.starts_with('$') => Some(Operation::Retrieve),
                // GET /ResourceType/id/CompartmentType -> compartment search
                [_resource_type, _id, _compartment] => Some(Operation::Query),
                _ => Some(Operation::Retrieve),
            },
            "POST" => match parts.as_slice() {
                // POST / -> batch/transaction
                [] => Some(Operation::Store),
                // POST /_search -> whole-system search
                ["_search"] => Some(Operation::Query),
                // POST /ResourceType/_search -> search (Query)
                [_resource_type, "_search"] => Some(Operation::Query),
                // POST /ResourceType/$operation -> type-level operation
                [_resource_type, op] if op.starts_with('$') => Some(Operation::Store),
                // POST /ResourceType/id/$operation -> instance-level operation
                [_resource_type, _id, op] if op.starts_with('$') => Some(Operation::Store),
                // POST /ResourceType -> create (Store)
                _ => Some(Operation::Store),
            },
            "PUT" => Some(Operation::Update),
            "PATCH" => Some(Operation::Patch),
            "DELETE" => Some(Operation::Delete),
            "HEAD" | "OPTIONS" => Some(Operation::Ping),
            _ => None,
        };

        if let Some(op) = operation {
            envelope.command = Some(Command::new(op).with_source("fhir_endpoint"));
        }

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
                "FHIR backend requires 'base_url' in options or valid 'connection' config",
            ));
        }

        // Use target_details from executor (already populated from request_details + backend config)
        // Fill in base_url if not already set by executor
        let mut target_details = if let Some(mut target) = envelope.target_details.take() {
            if target.base_url.is_empty() {
                target.base_url = base_url.to_string();
            }
            // Ensure FHIR-specific Accept header is set
            target
                .headers
                .entry("accept".to_string())
                .or_insert_with(|| "application/fhir+json".to_string());
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
                .headers
                .entry("accept".to_string())
                .or_insert_with(|| "application/fhir+json".to_string());
            target
        };

        // If an explicit Command is set on the envelope, use it to override the HTTP method
        // This enables protocol-bridging scenarios (e.g., DICOM -> FHIR)
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
                    "FHIR backend: Command({}) overriding method {} -> {}",
                    cmd.operation,
                    target_details.method,
                    overridden_method
                );
                target_details.method = overridden_method.to_string();
            }
        }

        // Apply path_prefix from backend options if present
        // This prepends a path to the request URI (e.g., "/fhir" + "/Patient" = "/fhir/Patient")
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
                "FHIR backend applied path_prefix '{}' to URI: {}",
                prefix,
                crate::logging::without_query(&target_details.uri)
            );
        }

        tracing::debug!(
            "FHIR backend targeting: {} {}",
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

        let (status, mut response_headers, body_bytes) = if use_http3 {
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

        // Ensure FHIR content type is set in response
        response_headers
            .entry("content-type".to_string())
            .or_insert_with(|| "application/fhir+json".to_string());

        tracing::debug!(
            "FHIR backend response body size: {} bytes",
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

        // Try to parse response as JSON if content-type indicates FHIR or JSON
        if let Some(content_type) = response_envelope
            .response_details
            .headers
            .get("content-type")
        {
            if content_type.contains("application/fhir+json")
                || content_type.contains("application/json")
            {
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
        envelope: &mut ResponseEnvelope<Vec<u8>>,
        ctx: &crate::models::protocol::ProtocolCtx,
        _options: &HashMap<String, Value>,
    ) -> Result<(), Error> {
        // Add protocol metadata and ensure FHIR content-type is set
        envelope
            .response_details
            .metadata
            .insert("protocol".to_string(), format!("{:?}", ctx.protocol));
        envelope
            .response_details
            .metadata
            .insert("service".to_string(), "fhir".to_string());

        // Ensure FHIR content-type is present for HTTP
        if ctx.protocol == crate::models::protocol::Protocol::Http {
            envelope
                .response_details
                .headers
                .entry("content-type".to_string())
                .or_insert_with(|| "application/fhir+json".to_string());
        }
        Ok(())
    }

    // Convert the processed ResponseEnvelope into an HTTP Response
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
                .map_err(|_| Error::from("Failed to serialize FHIR response JSON"))?;
            Body::from(body_bytes)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct FHIR HTTP response"))
    }
}

/// HTTP/1.1/2 and HTTP/3 request helpers for FhirEndpoint
impl FhirEndpoint {
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

        let mut headers = upstream_request_headers(
            &target_details.headers,
            &envelope.request_details.headers,
            InboundAuthorizationPolicy::from_backend_options(options)?,
            "FHIR",
        );
        // RequestBuilder::header appends, so the target's credential must go
        // into the map, where it replaces a same-named header in any letter case.
        crate::models::services::backend_auth::apply_backend_authentication_to_headers(
            &mut headers,
            options,
            "FHIR",
        );
        for (key, value) in headers {
            request_builder = request_builder.header(key, value);
        }

        // Ensure Accept is set to FHIR JSON
        request_builder = request_builder.header("Accept", "application/fhir+json");

        // Add request body if present
        if !envelope.original_data.is_empty() {
            request_builder = request_builder.body(envelope.original_data.clone());
        }

        tracing::debug!(
            "Sending FHIR request to: {}",
            crate::logging::without_query(&full_url)
        );

        // Execute the request
        let response = request_builder
            .send()
            .await
            // reqwest appends the full URL, search values included, to its
            // error text, and the router logs that text at ERROR.
            .map_err(|e| Error::from(format!("FHIR request failed: {}", e.without_url())))?;

        let status = response.status().as_u16();
        tracing::debug!("FHIR backend response status: {}", status);

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
                    "Failed to read FHIR response body: {}",
                    e.without_url()
                ))
            })?
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
            "FHIR HTTP/3",
        );
        // Target authentication: the inbound Authorization was already dropped above, so
        // the target's credential is the only one sent.
        crate::models::services::backend_auth::apply_backend_authentication_to_headers(
            &mut headers,
            options,
            "FHIR HTTP/3",
        );
        // Ensure Accept is set to FHIR JSON
        headers
            .entry("accept".to_string())
            .or_insert_with(|| "application/fhir+json".to_string());

        tracing::debug!(
            "Sending FHIR HTTP/3 request to: {}:{}{}",
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
            .map_err(|e| Error::from(format!("FHIR HTTP/3 request failed: {}", e)))?;

        let status = response.status.as_u16();
        tracing::debug!("FHIR HTTP/3 backend response status: {}", status);

        Ok((status, response.headers, response.body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::RequestDetails;

    #[test]
    fn test_validate_with_path_prefix() {
        let endpoint = FhirEndpoint {};
        let mut options = HashMap::new();
        options.insert("path_prefix".to_string(), serde_json::json!("/fhir"));
        assert!(endpoint.validate(&options).is_ok());
    }

    #[test]
    fn test_validate_with_connection_base_path() {
        let endpoint = FhirEndpoint {};
        let mut options = HashMap::new();
        let connection = ConnectionConfig {
            base_path: Some("/fhir".to_string()),
            ..Default::default()
        };
        options.insert(
            "connection".to_string(),
            serde_json::to_value(connection).unwrap(),
        );
        assert!(endpoint.validate(&options).is_ok());
    }

    #[test]
    fn test_validate_missing_both() {
        // path_prefix and connection.base_path are OPTIONAL per DSL schema
        // Services should default to a reasonable path if not provided
        let endpoint = FhirEndpoint {};
        let options = HashMap::new();
        assert!(endpoint.validate(&options).is_ok());
    }

    #[test]
    fn test_build_router_priority() {
        let endpoint = FhirEndpoint {};
        let mut options = HashMap::new();
        options.insert("path_prefix".to_string(), serde_json::json!("/primary"));
        let connection = ConnectionConfig {
            base_path: Some("/secondary".to_string()),
            ..Default::default()
        };
        options.insert(
            "connection".to_string(),
            serde_json::to_value(connection).unwrap(),
        );

        let routes = endpoint.build_router(&options);
        assert_eq!(routes.len(), 1);
        assert!(routes[0].path.starts_with("/primary"));
    }

    #[test]
    fn test_build_router_fallback() {
        let endpoint = FhirEndpoint {};
        let mut options = HashMap::new();
        let connection = ConnectionConfig {
            base_path: Some("/fallback".to_string()),
            ..Default::default()
        };
        options.insert(
            "connection".to_string(),
            serde_json::to_value(connection).unwrap(),
        );

        let routes = endpoint.build_router(&options);
        assert_eq!(routes.len(), 1);
        assert!(routes[0].path.starts_with("/fallback"));
    }

    /// FHIR search parameters carry patient values, so the query string must
    /// not reach the log. Port 1 refuses the connection, after the URL is logged.
    #[tokio::test]
    async fn debug_log_does_not_contain_query_string() {
        let marker = "DOE-PHI-MARKER";
        let endpoint = FhirEndpoint {};
        let mut options = HashMap::new();
        let connection = ConnectionConfig {
            host: "127.0.0.1".to_string(),
            protocol: Some("http".to_string()),
            port: Some(1),
            base_path: None,
            ca_cert_path: None,
        };
        options.insert(
            "connection".to_string(),
            serde_json::to_value(connection).unwrap(),
        );
        options.insert("path_prefix".to_string(), serde_json::json!("/fhir"));

        let mut query_params = HashMap::new();
        query_params.insert("name".to_string(), vec![marker.to_string()]);
        query_params.insert("birthdate".to_string(), vec!["1970-01-01".to_string()]);
        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/Patient".to_string(),
            query_params,
            ..Default::default()
        };
        let envelope = RequestEnvelope::new(request_details, vec![]);

        let (log, _guard) = crate::logging::test_capture::capture_debug();
        let _ = endpoint.backend_outgoing_request(envelope, &options).await;

        let out = log.contents();
        assert!(
            out.contains("/fhir/Patient"),
            "path should be logged: {out}"
        );
        assert!(!out.contains(marker), "query values leaked into log: {out}");
        assert!(
            !out.contains("birthdate"),
            "query string leaked into log: {out}"
        );
    }

    /// Builds a FHIR search request with a patient name and birthdate, sent to
    /// a plain-HTTP backend on `port`.
    fn search_request_to(
        port: u16,
        marker: &str,
    ) -> (RequestEnvelope<Vec<u8>>, HashMap<String, serde_json::Value>) {
        let mut options = HashMap::new();
        let connection = ConnectionConfig {
            host: "127.0.0.1".to_string(),
            protocol: Some("http".to_string()),
            port: Some(port),
            base_path: None,
            ca_cert_path: None,
        };
        options.insert(
            "connection".to_string(),
            serde_json::to_value(connection).unwrap(),
        );

        let mut query_params = HashMap::new();
        query_params.insert("name".to_string(), vec![marker.to_string()]);
        query_params.insert("birthdate".to_string(), vec!["1970-01-01".to_string()]);
        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/Patient".to_string(),
            query_params,
            ..Default::default()
        };
        (RequestEnvelope::new(request_details, vec![]), options)
    }

    /// reqwest appends the full URL, query included, to its error text. The
    /// router logs that text at ERROR, the default level.
    #[tokio::test]
    async fn send_error_does_not_contain_search_values() {
        let marker = "DOE-PHI-MARKER";
        let (envelope, options) = search_request_to(1, marker);

        let err = FhirEndpoint {}
            .backend_outgoing_request(envelope, &options)
            .await
            .err()
            .expect("port 1 must refuse the connection");

        let text = format!("{} {:?}", err, err);
        assert!(text.contains("FHIR request failed"), "error: {text}");
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
        let (envelope, options) = search_request_to(port, marker);

        let err = FhirEndpoint {}
            .backend_outgoing_request(envelope, &options)
            .await
            .err()
            .expect("a truncated body must fail");

        let text = format!("{} {:?}", err, err);
        assert!(
            text.contains("Failed to read FHIR response body"),
            "error: {text}"
        );
        assert!(!text.contains(marker), "search values leaked: {text}");
        assert!(!text.contains("birthdate"), "query string leaked: {text}");
    }

    #[tokio::test]
    async fn test_backend_outgoing_connection_url() {
        let endpoint = FhirEndpoint {};
        let mut options = HashMap::new();
        let connection = ConnectionConfig {
            host: "example.com".to_string(),
            protocol: Some("https".to_string()),
            port: Some(8443),
            base_path: Some("/r4".to_string()),
            ca_cert_path: None,
        };
        options.insert(
            "connection".to_string(),
            serde_json::to_value(connection).unwrap(),
        );

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/Patient/123".to_string(),
            ..Default::default()
        };
        let envelope = RequestEnvelope::new(request_details, vec![]);

        let result = endpoint.backend_outgoing_request(envelope, &options).await;
        // It will fail to connect, but we check if the error message implies it tried the correct URL
        // OR we can check the target details if we could intercept it.
        // Since we can't easily mock the HTTP client here, we'll rely on the fact that it *tries* to send.
        // However, we can check if it fails with a specific error related to connection, not configuration.
        match result {
            Err(e) => {
                // If it failed due to config, the error would be explicit.
                // If it tried to connect, it means URL construction worked.
                assert!(!e.to_string().contains("requires 'base_url'"));
            }
            _ => {}
        }
    }
}
