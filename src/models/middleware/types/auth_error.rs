use thiserror::Error;

/// Authentication failure error type for middleware
///
/// Carry a message for compatibility with existing tests and logs.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct AuthFailure(pub &'static str);

/// Authentication could not be checked because the key source failed
///
/// Returns HTTP 503 Service Unavailable. Used when a JWKS endpoint is
/// unreachable, returns an error status, or serves an unusable document or
/// key. The caller's token may be valid, so 401 would wrongly blame the client.
#[derive(Debug, Error)]
#[error("Authentication unavailable: {0}")]
pub struct AuthUnavailable(pub String);
