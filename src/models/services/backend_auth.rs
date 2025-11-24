//! Backend authentication helper for HTTP-based services
//!
//! This module provides common authentication functionality for HTTP and FHIR backends.

use crate::models::connection::AuthenticationDefinition;
use base64::{engine::general_purpose, Engine as _};
use std::collections::HashMap;

/// Apply authentication headers to a reqwest RequestBuilder
///
/// Supports the following authentication methods:
/// - `basic`: Base64-encoded username:password in Authorization header
/// - `bearer`: Bearer token in Authorization header
/// - `api_key`: Custom header (default X-API-Key) with API key value
/// - `none` or empty: No authentication
///
/// # Arguments
/// * `request_builder` - The reqwest RequestBuilder to apply headers to
/// * `options` - Backend options that may contain `authentication_def`
/// * `service_name` - Service name for logging (e.g., "HTTP", "FHIR")
///
/// # Returns
/// The RequestBuilder with authentication headers applied
pub fn apply_backend_authentication(
    mut request_builder: reqwest::RequestBuilder,
    options: &HashMap<String, serde_json::Value>,
    service_name: &str,
) -> reqwest::RequestBuilder {
    if let Some(auth_def_json) = options.get("authentication_def") {
        if let Ok(auth_def) =
            serde_json::from_value::<AuthenticationDefinition>(auth_def_json.clone())
        {
            match auth_def.method.as_str() {
                "basic" => {
                    // Basic Auth: username:password in base64
                    if let (Some(username), Some(password)) = (
                        auth_def.options.get("username").and_then(|v| v.as_str()),
                        auth_def.options.get("password").and_then(|v| v.as_str()),
                    ) {
                        let credentials = format!("{}:{}", username, password);
                        let encoded = general_purpose::STANDARD.encode(credentials.as_bytes());
                        request_builder =
                            request_builder.header("Authorization", format!("Basic {}", encoded));
                        tracing::debug!(
                            "Applied Basic authentication for {} backend request",
                            service_name
                        );
                    }
                }
                "bearer" => {
                    // Bearer Token
                    if let Some(token) = auth_def.options.get("token").and_then(|v| v.as_str()) {
                        request_builder =
                            request_builder.header("Authorization", format!("Bearer {}", token));
                        tracing::debug!(
                            "Applied Bearer authentication for {} backend request",
                            service_name
                        );
                    }
                }
                "api_key" => {
                    // API Key in custom header
                    let header_name = auth_def
                        .options
                        .get("header_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("X-API-Key");
                    if let Some(api_key) = auth_def.options.get("api_key").and_then(|v| v.as_str())
                    {
                        request_builder = request_builder.header(header_name, api_key);
                        tracing::debug!(
                            "Applied API Key authentication ({}) for {} backend request",
                            header_name,
                            service_name
                        );
                    }
                }
                "none" | "" => {
                    // No authentication
                    tracing::debug!("No authentication configured for {} backend", service_name);
                }
                other => {
                    tracing::warn!(
                        "Unsupported authentication method for {} backend: {}",
                        service_name,
                        other
                    );
                }
            }
        }
    }

    request_builder
}
