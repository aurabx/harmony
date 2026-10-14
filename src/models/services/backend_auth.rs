//! Backend authentication helper for HTTP-based services
//!
//! This module provides common authentication functionality for HTTP and FHIR backends.

use crate::models::connection::AuthenticationDefinition;
use base64::{engine::general_purpose, Engine as _};
use std::collections::HashMap;

/// Return true when the backend options carry an authentication method of their own.
///
/// This mirrors what [`apply_backend_authentication`] reads: an
/// `authentication_def` that parses and names a method other than `none` or
/// an empty string. The upstream header filter uses it so the target's
/// credential wins over one the client sent.
pub fn has_backend_authentication(options: &HashMap<String, serde_json::Value>) -> bool {
    options
        .get("authentication_def")
        .and_then(|value| serde_json::from_value::<AuthenticationDefinition>(value.clone()).ok())
        .is_some_and(|auth_def| !matches!(auth_def.method.as_str(), "none" | ""))
}

/// Build the credential headers a backend's target authentication adds to a request.
///
/// Supports the following authentication methods:
/// - `basic`: Base64-encoded username:password in Authorization header
/// - `bearer`: Bearer token in Authorization header
/// - `api_key`: Custom header (default X-API-Key) with API key value
/// - `none` or empty: No authentication
///
/// Transports that do not use reqwest (HTTP/3) call this directly. The reqwest
/// path goes through [`apply_backend_authentication`], so both read the same rules.
///
/// # Arguments
/// * `options` - Backend options that may contain `authentication_def`
/// * `service_name` - Service name for logging (e.g., "HTTP", "FHIR")
///
/// # Returns
/// `(header name, header value)` pairs. Empty when the backend has no usable
/// authentication. Credential values are never logged.
pub fn backend_authentication_headers(
    options: &HashMap<String, serde_json::Value>,
    service_name: &str,
) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    let Some(auth_def_json) = options.get("authentication_def") else {
        return headers;
    };
    let Ok(auth_def) = serde_json::from_value::<AuthenticationDefinition>(auth_def_json.clone())
    else {
        return headers;
    };

    match auth_def.method.as_str() {
        "basic" => {
            // Basic Auth: username:password in base64
            if let (Some(username), Some(password)) = (
                auth_def.options.get("username").and_then(|v| v.as_str()),
                auth_def.options.get("password").and_then(|v| v.as_str()),
            ) {
                let credentials = format!("{}:{}", username, password);
                let encoded = general_purpose::STANDARD.encode(credentials.as_bytes());
                headers.push(("Authorization".to_string(), format!("Basic {}", encoded)));
                tracing::debug!(
                    "Applied Basic authentication for {} backend request",
                    service_name
                );
            }
        }
        "bearer" => {
            if let Some(token) = auth_def.options.get("token").and_then(|v| v.as_str()) {
                headers.push(("Authorization".to_string(), format!("Bearer {}", token)));
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
            if let Some(api_key) = auth_def.options.get("api_key").and_then(|v| v.as_str()) {
                headers.push((header_name.to_string(), api_key.to_string()));
                tracing::debug!(
                    "Applied API Key authentication ({}) for {} backend request",
                    header_name,
                    service_name
                );
            }
        }
        "none" | "" => {
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

    headers
}

/// Apply the target's authentication headers to a reqwest RequestBuilder.
///
/// See [`backend_authentication_headers`] for the supported methods.
pub fn apply_backend_authentication(
    mut request_builder: reqwest::RequestBuilder,
    options: &HashMap<String, serde_json::Value>,
    service_name: &str,
) -> reqwest::RequestBuilder {
    for (name, value) in backend_authentication_headers(options, service_name) {
        request_builder = request_builder.header(name, value);
    }
    request_builder
}

/// Add the target's credential to an HTTP/3 request's header map.
///
/// A header the client sent under the same name, in any letter case, is
/// replaced. The map holds names as the client wrote them, so a plain insert
/// could leave two entries that differ only by case and send both upstream.
pub fn apply_backend_authentication_to_headers(
    headers: &mut HashMap<String, String>,
    options: &HashMap<String, serde_json::Value>,
    service_name: &str,
) {
    for (name, value) in backend_authentication_headers(options, service_name) {
        headers.retain(|existing, _| !existing.eq_ignore_ascii_case(&name));
        headers.insert(name, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn options_with(
        method: &str,
        auth_options: serde_json::Value,
    ) -> HashMap<String, serde_json::Value> {
        let mut options = HashMap::new();
        options.insert(
            "authentication_def".to_string(),
            json!({ "id": "test-auth", "method": method, "options": auth_options }),
        );
        options
    }

    #[test]
    fn basic_builds_authorization_header() {
        let options = options_with("basic", json!({ "username": "svc", "password": "pw" }));
        let headers = backend_authentication_headers(&options, "TEST");
        assert_eq!(
            headers,
            vec![("Authorization".to_string(), "Basic c3ZjOnB3".to_string())]
        );
    }

    #[test]
    fn bearer_builds_authorization_header() {
        let options = options_with("bearer", json!({ "token": "abc" }));
        let headers = backend_authentication_headers(&options, "TEST");
        assert_eq!(
            headers,
            vec![("Authorization".to_string(), "Bearer abc".to_string())]
        );
    }

    #[test]
    fn api_key_uses_default_and_custom_header_names() {
        let default = options_with("api_key", json!({ "api_key": "k1" }));
        assert_eq!(
            backend_authentication_headers(&default, "TEST"),
            vec![("X-API-Key".to_string(), "k1".to_string())]
        );

        let custom = options_with(
            "api_key",
            json!({ "api_key": "k2", "header_name": "X-Token" }),
        );
        assert_eq!(
            backend_authentication_headers(&custom, "TEST"),
            vec![("X-Token".to_string(), "k2".to_string())]
        );
    }

    #[test]
    fn none_unknown_incomplete_and_missing_add_nothing() {
        assert!(
            backend_authentication_headers(&options_with("none", json!({})), "TEST").is_empty()
        );
        assert!(
            backend_authentication_headers(&options_with("digest", json!({})), "TEST").is_empty()
        );
        assert!(backend_authentication_headers(
            &options_with("basic", json!({ "username": "svc" })),
            "TEST"
        )
        .is_empty());
        assert!(backend_authentication_headers(&HashMap::new(), "TEST").is_empty());
    }

    #[test]
    fn target_credential_replaces_client_header_in_any_case() {
        let options = options_with("bearer", json!({ "token": "target" }));
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Bearer client".to_string());
        headers.insert("x-keep".to_string(), "1".to_string());

        apply_backend_authentication_to_headers(&mut headers, &options, "TEST");

        assert_eq!(headers.len(), 2);
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer target")
        );
        assert_eq!(headers.get("x-keep").map(String::as_str), Some("1"));
    }
}
