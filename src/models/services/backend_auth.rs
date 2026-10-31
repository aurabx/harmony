//! Backend authentication helper for HTTP-based services
//!
//! This module provides common authentication functionality for HTTP and FHIR backends.

use crate::models::connection::AuthenticationDefinition;
use base64::{engine::general_purpose, Engine as _};
use std::collections::HashMap;
use std::fmt;

/// Methods whose credential a backend can send as a request header.
pub const HEADER_CREDENTIAL_METHODS: &[&str] = &["basic", "bearer", "api_key"];

/// Method that sends no credential.
pub const NO_CREDENTIAL_METHOD: &str = "none";

/// Method for definitions that hold JWT validation settings. Only middleware
/// (jwt_auth) can use it; a backend has no way to send it.
pub const JWT_METHOD: &str = "jwt";

/// Headers that control how a request is framed or delivered, compared in
/// lowercase. An `api_key` credential sent under one of these names would
/// corrupt the request. `Proxy-Authorization` is not here: sending a
/// credential to an authenticating upstream proxy under it is a valid setup.
const RESERVED_API_KEY_HEADERS: &[&str] = &[
    "host",
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
];

/// Matches the limit of reqwest's default redirect policy.
const MAX_REDIRECTS: usize = 10;

/// Header used by `api_key` when the definition sets no `header_name`.
const DEFAULT_API_KEY_HEADER: &str = "X-API-Key";

/// Why an authentication definition cannot produce credential headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// A required option is absent, not a string, or an empty string.
    MissingOption {
        method: String,
        option: &'static str,
    },
    /// `api_key` names a `header_name` that is not a valid HTTP header name.
    InvalidHeaderName { header_name: String },
    /// `api_key` names a header the upstream filter controls, such as
    /// `Host` or `Content-Length`. Sending a credential there would corrupt
    /// the request framing.
    ReservedHeaderName { header_name: String },
    /// An option's value cannot go into an HTTP header, for example a token
    /// read from a file with a trailing newline. The value is never stored,
    /// so it cannot reach an error message or a log.
    InvalidOptionValue {
        method: String,
        option: &'static str,
        reason: &'static str,
    },
    /// The method is not one a backend can send.
    UnsupportedMethod { method: String },
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialError::MissingOption { method, option } => write!(
                f,
                "method '{}' requires a non-empty '{}' option",
                method.escape_default(),
                option
            ),
            CredentialError::InvalidHeaderName { header_name } => write!(
                f,
                "'header_name' value '{}' is not a valid HTTP header name",
                header_name.escape_default()
            ),
            CredentialError::ReservedHeaderName { header_name } => write!(
                f,
                "'header_name' value '{}' is reserved for request framing and cannot carry a credential",
                header_name.escape_default()
            ),
            CredentialError::InvalidOptionValue {
                method,
                option,
                reason,
            } => write!(
                f,
                "method '{}' option '{}' {}",
                method.escape_default(),
                option,
                reason
            ),
            CredentialError::UnsupportedMethod { method } => write!(
                f,
                "method '{}' cannot be sent by a backend. Use one of: {}, {}",
                method.escape_default(),
                HEADER_CREDENTIAL_METHODS.join(", "),
                NO_CREDENTIAL_METHOD
            ),
        }
    }
}

/// Build the credential headers a definition sends upstream.
///
/// This is the single rule set for backend credentials. Config load calls it
/// to reject a definition that could not send a credential, and request
/// handling calls it to build the headers, so the two cannot disagree.
///
/// Returns an empty list for `none`. Returns an error for a missing or empty
/// required option, a value that cannot go into a header, an invalid or
/// reserved `api_key` header name, or a method a backend cannot send. Never
/// logs, so it is safe to call on every request.
pub fn credential_headers(
    auth_def: &AuthenticationDefinition,
) -> Result<Vec<(String, String)>, CredentialError> {
    match auth_def.method.as_str() {
        "basic" => {
            let username = required_option(auth_def, "username")?;
            let password = required_option(auth_def, "password")?;
            // RFC 7617: the user-id cannot contain a colon, because the
            // server splits user-id and password at the first one.
            if username.contains(':') {
                return Err(CredentialError::InvalidOptionValue {
                    method: auth_def.method.clone(),
                    option: "username",
                    reason: "must not contain ':'",
                });
            }
            let encoded =
                general_purpose::STANDARD.encode(format!("{}:{}", username, password).as_bytes());
            Ok(vec![(
                "Authorization".to_string(),
                format!("Basic {}", encoded),
            )])
        }
        "bearer" => {
            let token = header_safe_option(auth_def, "token")?;
            Ok(vec![(
                "Authorization".to_string(),
                format!("Bearer {}", token),
            )])
        }
        "api_key" => {
            let api_key = header_safe_option(auth_def, "api_key")?;
            let header_name = match auth_def.options.get("header_name") {
                None => DEFAULT_API_KEY_HEADER.to_string(),
                Some(value) => {
                    let name = value.as_str().unwrap_or_default();
                    if http::HeaderName::from_bytes(name.as_bytes()).is_err() {
                        return Err(CredentialError::InvalidHeaderName {
                            header_name: value
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| value.to_string()),
                        });
                    }
                    if RESERVED_API_KEY_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                        return Err(CredentialError::ReservedHeaderName {
                            header_name: name.to_string(),
                        });
                    }
                    name.to_string()
                }
            };
            Ok(vec![(header_name, api_key.to_string())])
        }
        NO_CREDENTIAL_METHOD => Ok(Vec::new()),
        other => Err(CredentialError::UnsupportedMethod {
            method: other.to_string(),
        }),
    }
}

/// Read a string option that must be present and not blank.
///
/// A whitespace-only value counts as missing. It usually means an unset
/// environment variable next to a stray space, such as `"$TOKEN "`.
fn required_option<'a>(
    auth_def: &'a AuthenticationDefinition,
    option: &'static str,
) -> Result<&'a str, CredentialError> {
    auth_def
        .options
        .get(option)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CredentialError::MissingOption {
            method: auth_def.method.clone(),
            option,
        })
}

/// Read a required option that is sent as a header value as-is.
///
/// Rejects control characters such as a trailing newline. Load accepting
/// such a value would make every request fail at send time with an error
/// that does not name the cause.
fn header_safe_option<'a>(
    auth_def: &'a AuthenticationDefinition,
    option: &'static str,
) -> Result<&'a str, CredentialError> {
    let value = required_option(auth_def, option)?;
    if http::HeaderValue::from_str(value).is_err() || value.chars().any(char::is_control) {
        return Err(CredentialError::InvalidOptionValue {
            method: auth_def.method.clone(),
            option,
            reason: "contains characters that are not allowed in an HTTP header value, such as a newline",
        });
    }
    Ok(value)
}

/// Return true when the backend options carry a credential Harmony will send.
///
/// The upstream header filter uses it so the target's credential wins over
/// one the client sent. It uses [`credential_headers`], so a definition that
/// builds no header never causes the client's credential to be dropped.
pub fn has_backend_authentication(options: &HashMap<String, serde_json::Value>) -> bool {
    options
        .get("authentication_def")
        .and_then(|value| serde_json::from_value::<AuthenticationDefinition>(value.clone()).ok())
        .and_then(|auth_def| credential_headers(&auth_def).ok())
        .is_some_and(|headers| !headers.is_empty())
}

/// Redirect policy for a reqwest client that sends a request upstream.
///
/// On a redirect, reqwest removes `Authorization` when the host or port changes,
/// but it always keeps an `api_key` header. No client follows a move from https
/// to http, because that would send the request and any credential in clear
/// text. When Harmony sends a target credential, a redirect to another origin
/// is not followed either. The 3xx response then goes back to the caller.
pub fn upstream_redirect_policy(
    options: &HashMap<String, serde_json::Value>,
) -> reqwest::redirect::Policy {
    let credentialed = has_backend_authentication(options);
    reqwest::redirect::Policy::custom(move |attempt| {
        let Some(first) = attempt.previous().first() else {
            return attempt.stop();
        };
        let downgrade = first.scheme() == "https" && attempt.url().scheme() != "https";
        let other_origin = first.origin() != attempt.url().origin();
        if downgrade || (credentialed && other_origin) {
            return attempt.stop();
        }
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.stop();
        }
        attempt.follow()
    })
}

/// Build the credential headers a backend's target authentication adds to a request.
///
/// See [`credential_headers`] for the supported methods and rules. Config load
/// rejects definitions that cannot build a credential, so the error branches
/// here mean a definition bypassed load-time checks. They log at `error`
/// because the request then goes upstream without the credential.
///
/// Backends that forward client headers go through
/// [`apply_backend_authentication_to_headers`]. Requests that carry no client
/// headers go through [`apply_backend_authentication`]. Both read the same rules.
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
    let Some(auth_def_json) = options.get("authentication_def") else {
        return Vec::new();
    };
    let auth_def = match serde_json::from_value::<AuthenticationDefinition>(auth_def_json.clone()) {
        Ok(auth_def) => auth_def,
        Err(e) => {
            tracing::error!(
                "{} backend: authentication definition could not be read, sending no credential: {}",
                service_name,
                e
            );
            return Vec::new();
        }
    };

    match credential_headers(&auth_def) {
        Ok(headers) if headers.is_empty() => {
            tracing::debug!("No authentication configured for {} backend", service_name);
            headers
        }
        Ok(headers) => {
            tracing::debug!(
                "Applied {} authentication for {} backend request",
                auth_def.method,
                service_name
            );
            headers
        }
        Err(e) => {
            tracing::error!(
                "{} backend: authentication '{}' is unusable, sending no credential: {}",
                service_name,
                auth_def.id.escape_default(),
                e
            );
            Vec::new()
        }
    }
}

/// Apply the target's authentication headers to a reqwest RequestBuilder.
///
/// `RequestBuilder::header` appends, so this suits only a request that carries
/// no client headers. A backend that forwards client headers must use
/// [`apply_backend_authentication_to_headers`], or a client could send a second
/// value for the credential header.
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

/// Add the target's credential to the header map a backend sends upstream.
///
/// A header the client sent under the same name, in any letter case, is
/// replaced. The map holds names as the client wrote them, so a plain insert
/// could leave two entries that differ only by case and send both upstream.
/// A name that differs only by `_` for `-` is replaced too, because CGI-style
/// upstreams read `X_API_KEY` and `X-API-Key` as one header.
pub fn apply_backend_authentication_to_headers(
    headers: &mut HashMap<String, String>,
    options: &HashMap<String, serde_json::Value>,
    service_name: &str,
) {
    for (name, value) in backend_authentication_headers(options, service_name) {
        headers.retain(|existing, _| !names_same_header(existing, &name));
        headers.insert(name, value);
    }
}

/// Compare header names, ignoring letter case and treating `_` as `-`.
fn names_same_header(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left.bytes().zip(right.bytes()).all(|(a, b)| {
            let canonical = |byte: u8| if byte == b'_' { b'-' } else { byte };
            canonical(a).eq_ignore_ascii_case(&canonical(b))
        })
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

    #[test]
    fn target_credential_replaces_underscore_variant() {
        let options = options_with("api_key", json!({ "api_key": "target" }));
        let mut headers = HashMap::new();
        headers.insert("x_api_key".to_string(), "client".to_string());
        headers.insert("x-apikey".to_string(), "kept".to_string());

        apply_backend_authentication_to_headers(&mut headers, &options, "TEST");

        assert_eq!(headers.len(), 2);
        assert_eq!(headers.get("X-API-Key").map(String::as_str), Some("target"));
        assert_eq!(headers.get("x-apikey").map(String::as_str), Some("kept"));
    }

    fn definition(method: &str, auth_options: serde_json::Value) -> AuthenticationDefinition {
        serde_json::from_value(
            json!({ "id": "test-auth", "method": method, "options": auth_options }),
        )
        .expect("valid definition")
    }

    #[test]
    fn credential_headers_reports_each_missing_or_empty_option() {
        let cases = [
            ("bearer", json!({}), "token"),
            ("bearer", json!({ "token": "" }), "token"),
            ("bearer", json!({ "token": 5 }), "token"),
            ("bearer", json!({ "token": "  " }), "token"),
            ("basic", json!({ "password": "pw" }), "username"),
            ("basic", json!({ "username": "svc" }), "password"),
            (
                "basic",
                json!({ "username": "svc", "password": "" }),
                "password",
            ),
            ("api_key", json!({ "header_name": "X-Token" }), "api_key"),
        ];
        for (method, auth_options, option) in cases {
            assert_eq!(
                credential_headers(&definition(method, auth_options.clone())),
                Err(CredentialError::MissingOption {
                    method: method.to_string(),
                    option
                }),
                "{} with {}",
                method,
                auth_options
            );
        }
    }

    #[test]
    fn credential_headers_rejects_invalid_api_key_header_name() {
        for header_name in [json!("X Token"), json!(""), json!("X-Token\n"), json!(7)] {
            let result = credential_headers(&definition(
                "api_key",
                json!({ "api_key": "k", "header_name": header_name }),
            ));
            assert!(
                matches!(result, Err(CredentialError::InvalidHeaderName { .. })),
                "{:?}",
                result
            );
        }
    }

    #[test]
    fn credential_headers_rejects_methods_a_backend_cannot_send() {
        for method in ["jwt", "digest", ""] {
            assert_eq!(
                credential_headers(&definition(method, json!({}))),
                Err(CredentialError::UnsupportedMethod {
                    method: method.to_string()
                })
            );
        }
        assert_eq!(
            credential_headers(&definition("none", json!({}))),
            Ok(Vec::new())
        );
    }

    #[test]
    fn credential_error_escapes_control_characters() {
        let message = CredentialError::UnsupportedMethod {
            method: "x\nERROR: forged \u{1b}[31m".to_string(),
        }
        .to_string();
        assert!(!message.chars().any(char::is_control), "{:?}", message);
    }

    #[test]
    fn has_backend_authentication_requires_a_buildable_credential() {
        assert!(has_backend_authentication(&options_with(
            "bearer",
            json!({ "token": "t" })
        )));
        assert!(!has_backend_authentication(&options_with(
            "bearer",
            json!({})
        )));
        assert!(!has_backend_authentication(&options_with("jwt", json!({}))));
        assert!(!has_backend_authentication(&options_with(
            "none",
            json!({})
        )));
        assert!(!has_backend_authentication(&HashMap::new()));
    }

    #[test]
    fn credential_headers_rejects_values_that_cannot_be_header_values() {
        let cases = [
            ("bearer", json!({ "token": "abc\n" }), "token"),
            ("bearer", json!({ "token": "a\tb" }), "token"),
            (
                "api_key",
                json!({ "api_key": "k\r\nX-Injected: 1" }),
                "api_key",
            ),
            (
                "basic",
                json!({ "username": "a:b", "password": "pw" }),
                "username",
            ),
        ];
        for (method, auth_options, option) in cases {
            let result = credential_headers(&definition(method, auth_options.clone()));
            match result {
                Err(CredentialError::InvalidOptionValue {
                    option: reported, ..
                }) => assert_eq!(reported, option, "{}", auth_options),
                other => panic!("{} with {}: {:?}", method, auth_options, other),
            }
        }
    }

    #[test]
    fn invalid_value_error_never_contains_the_value() {
        let error = credential_headers(&definition("bearer", json!({ "token": "s3cret\n" })))
            .expect_err("token has a newline");
        assert!(!error.to_string().contains("s3cret"), "{}", error);
    }

    #[test]
    fn credential_headers_rejects_reserved_api_key_header_names() {
        for header_name in [
            "Host",
            "content-length",
            "Transfer-Encoding",
            "Connection",
            "Keep-Alive",
            "TE",
            "Trailer",
            "Upgrade",
            "Proxy-Connection",
        ] {
            let result = credential_headers(&definition(
                "api_key",
                json!({ "api_key": "k", "header_name": header_name }),
            ));
            assert!(
                matches!(result, Err(CredentialError::ReservedHeaderName { .. })),
                "{}: {:?}",
                header_name,
                result
            );
        }
    }

    #[test]
    fn api_key_may_be_sent_as_proxy_authorization() {
        let result = credential_headers(&definition(
            "api_key",
            json!({ "api_key": "Basic cHJveHk6cHc=", "header_name": "Proxy-Authorization" }),
        ));
        assert_eq!(
            result,
            Ok(vec![(
                "Proxy-Authorization".to_string(),
                "Basic cHJveHk6cHc=".to_string()
            )])
        );
    }
}
