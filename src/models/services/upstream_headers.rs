//! Header filter for requests that backends send upstream.
//!
//! Every backend that builds an upstream HTTP request passes its candidate
//! headers through [`upstream_request_headers`]. The filter enforces one rule
//! set for all backends:
//!
//! - `Host`, the RFC 9110 §7.6.1 hop-by-hop headers, any header named in the
//!   inbound `Connection` header, and `Content-Length` are always removed. The
//!   HTTP client sets `Host` from the URL and `Content-Length` from the body it
//!   actually sends.
//! - The client's `Authorization` header is removed unless the backend sets
//!   `forward_authorization = true`. Even then it is removed when Harmony sends
//!   its own credential (target authentication or a middleware-set header such
//!   as the mesh JWT), so the upstream never receives two `Authorization`
//!   headers.
//! - An `Authorization` header that Harmony set (for example the mesh egress
//!   JWT) is kept.
//! - Every other header passes through unchanged.
//!
//! The filter does not add the target's credential. Each backend applies it to
//! the filtered map with
//! [`apply_backend_authentication_to_headers`](crate::models::services::backend_auth::apply_backend_authentication_to_headers),
//! which replaces any header of the same name. That covers a client copy of an
//! `api_key` header and an `Authorization` header set by middleware.

use crate::utils::Error;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Backend option that lets the client's `Authorization` header reach the upstream.
pub const FORWARD_AUTHORIZATION_OPTION: &str = "forward_authorization";

/// Headers that never go upstream, compared in lowercase.
///
/// `proxy-connection` is not in RFC 9110 but some clients still send it, and
/// the backends stripped it before this filter existed.
const ALWAYS_STRIPPED: &[&str] = &[
    "host",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
];

/// How a backend treats the `Authorization` header the client sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InboundAuthorizationPolicy {
    /// The backend set `forward_authorization = true`.
    pub forward: bool,
    /// The backend has its own authentication (`authentication_def`), so its
    /// credential wins over the client's.
    pub target_authenticates: bool,
}

impl InboundAuthorizationPolicy {
    /// Read the policy from a backend's options.
    ///
    /// `forward_authorization` defaults to `false` when absent. A value that
    /// is not a boolean is a configuration error, so the request fails instead
    /// of quietly choosing a credential behaviour.
    pub fn from_backend_options(options: &HashMap<String, Value>) -> Result<Self, Error> {
        let forward = match options.get(FORWARD_AUTHORIZATION_OPTION) {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(other) => {
                return Err(Error::from(format!(
                    "Backend option '{}' must be a boolean (true or false), got: {}",
                    FORWARD_AUTHORIZATION_OPTION, other
                )));
            }
        };
        Ok(Self {
            forward,
            target_authenticates: crate::models::services::backend_auth::has_backend_authentication(
                options,
            ),
        })
    }
}

/// Build the header set a backend sends upstream.
///
/// * `outgoing` - the candidate headers: normally `target_details.headers`,
///   which start as a copy of the client's headers and then carry any headers
///   middleware added.
/// * `inbound` - the headers the client sent (`request_details.headers`). An
///   `Authorization` entry in `outgoing` with the same value as an inbound
///   `Authorization` header is treated as the client's credential. Any other
///   `Authorization` value was set by Harmony and is kept.
/// * `service_name` - used only in log messages.
///
/// Header values are never logged.
pub fn upstream_request_headers(
    outgoing: &HashMap<String, String>,
    inbound: &HashMap<String, String>,
    policy: InboundAuthorizationPolicy,
    service_name: &str,
) -> HashMap<String, String> {
    let connection_named = connection_header_tokens(outgoing, inbound);
    let inbound_credentials: HashSet<&str> = header_values(inbound, "authorization").collect();
    let harmony_sets_authorization =
        header_values(outgoing, "authorization").any(|value| !inbound_credentials.contains(value));

    let mut forwarded = HashMap::with_capacity(outgoing.len());
    for (name, value) in outgoing {
        let lower = name.to_ascii_lowercase();
        if ALWAYS_STRIPPED.contains(&lower.as_str()) || connection_named.contains(&lower) {
            continue;
        }

        if lower == "authorization" && inbound_credentials.contains(value.as_str()) {
            if !policy.forward {
                tracing::debug!(
                    "{} backend: dropped inbound Authorization header ({} is not enabled)",
                    service_name,
                    FORWARD_AUTHORIZATION_OPTION
                );
                continue;
            }
            if policy.target_authenticates || harmony_sets_authorization {
                tracing::debug!(
                    "{} backend: dropped inbound Authorization header because Harmony sends its own credential",
                    service_name
                );
                continue;
            }
        }

        forwarded.insert(name.clone(), value.clone());
    }
    forwarded
}

/// Lowercase header names listed in any `Connection` header (RFC 9110 §7.6.1).
fn connection_header_tokens(
    outgoing: &HashMap<String, String>,
    inbound: &HashMap<String, String>,
) -> HashSet<String> {
    header_values(outgoing, "connection")
        .chain(header_values(inbound, "connection"))
        .flat_map(|value| value.split(','))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

/// Values of every header whose name matches `lower_name` case-insensitively.
fn header_values<'a>(
    headers: &'a HashMap<String, String>,
    lower_name: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
    headers
        .iter()
        .filter(move |(name, _)| name.eq_ignore_ascii_case(lower_name))
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn policy(forward: bool, target_authenticates: bool) -> InboundAuthorizationPolicy {
        InboundAuthorizationPolicy {
            forward,
            target_authenticates,
        }
    }

    #[test]
    fn strips_host_hop_by_hop_and_content_length() {
        let inbound = headers(&[
            ("host", "harmony.local"),
            ("connection", "keep-alive, X-Hop"),
            ("keep-alive", "timeout=5"),
            ("proxy-authorization", "Basic cHJveHk="),
            ("proxy-authenticate", "Basic"),
            ("proxy-connection", "keep-alive"),
            ("te", "trailers"),
            ("trailer", "Expires"),
            ("transfer-encoding", "chunked"),
            ("upgrade", "websocket"),
            ("content-length", "42"),
            ("x-hop", "named-in-connection"),
            ("accept", "application/json"),
            ("x-request-id", "abc"),
        ]);
        let out = upstream_request_headers(&inbound, &inbound, policy(false, false), "test");
        assert_eq!(
            out,
            headers(&[("accept", "application/json"), ("x-request-id", "abc")])
        );
    }

    #[test]
    fn inbound_authorization_dropped_by_default() {
        let inbound = headers(&[("authorization", "Basic Y2xpZW50OnB3"), ("accept", "*/*")]);
        let out = upstream_request_headers(&inbound, &inbound, policy(false, false), "test");
        assert_eq!(out, headers(&[("accept", "*/*")]));
    }

    #[test]
    fn inbound_authorization_forwarded_when_enabled_and_no_target_auth() {
        let inbound = headers(&[("authorization", "Bearer client")]);
        let out = upstream_request_headers(&inbound, &inbound, policy(true, false), "test");
        assert_eq!(out, headers(&[("authorization", "Bearer client")]));
    }

    #[test]
    fn inbound_authorization_dropped_when_target_authenticates() {
        let inbound = headers(&[("authorization", "Bearer client")]);
        let out = upstream_request_headers(&inbound, &inbound, policy(true, true), "test");
        assert!(out.is_empty());
    }

    #[test]
    fn harmony_set_authorization_is_kept_and_wins_over_inbound() {
        let inbound = headers(&[("authorization", "Bearer client")]);
        let outgoing = headers(&[
            ("authorization", "Bearer client"),
            ("Authorization", "Bearer mesh-jwt"),
        ]);

        for forward in [false, true] {
            let out = upstream_request_headers(&outgoing, &inbound, policy(forward, false), "test");
            assert_eq!(out, headers(&[("Authorization", "Bearer mesh-jwt")]));
        }
    }

    #[test]
    fn harmony_set_authorization_kept_without_inbound_credential() {
        let outgoing = headers(&[("Authorization", "Bearer mesh-jwt")]);
        let out =
            upstream_request_headers(&outgoing, &HashMap::new(), policy(false, false), "test");
        assert_eq!(out, outgoing);
    }

    #[test]
    fn policy_reads_forward_authorization_and_target_auth() {
        let empty = HashMap::new();
        assert_eq!(
            InboundAuthorizationPolicy::from_backend_options(&empty).unwrap(),
            policy(false, false)
        );

        let opts = HashMap::from([
            (FORWARD_AUTHORIZATION_OPTION.to_string(), json!(true)),
            (
                "authentication_def".to_string(),
                json!({"id": "a", "method": "bearer", "options": {"token": "t"}}),
            ),
        ]);
        assert_eq!(
            InboundAuthorizationPolicy::from_backend_options(&opts).unwrap(),
            policy(true, true)
        );
    }

    /// A definition that builds no header must not count as Harmony sending
    /// its own credential. Otherwise the forwarded client credential is
    /// dropped and the request goes upstream with none.
    #[test]
    fn incomplete_target_auth_keeps_forwarded_client_credential() {
        let opts = HashMap::from([
            (FORWARD_AUTHORIZATION_OPTION.to_string(), json!(true)),
            (
                "authentication_def".to_string(),
                json!({"id": "a", "method": "bearer", "options": {}}),
            ),
        ]);
        let policy = InboundAuthorizationPolicy::from_backend_options(&opts).unwrap();
        assert!(!policy.target_authenticates);

        let inbound = HashMap::from([("Authorization".to_string(), "Bearer client".to_string())]);
        let out = upstream_request_headers(&inbound, &inbound, policy, "test");
        assert_eq!(
            out.get("Authorization").map(String::as_str),
            Some("Bearer client")
        );
    }

    #[test]
    fn policy_rejects_non_boolean_forward_authorization() {
        let opts = HashMap::from([(FORWARD_AUTHORIZATION_OPTION.to_string(), json!("true"))]);
        let err = InboundAuthorizationPolicy::from_backend_options(&opts).unwrap_err();
        assert!(err.to_string().contains(FORWARD_AUTHORIZATION_OPTION));
    }
}
