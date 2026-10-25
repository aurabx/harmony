use thiserror::Error;

/// Path not found / path filter denial error type for middleware
///
/// Returns HTTP 404 Not Found when a path is denied by path filter rules.
#[derive(Debug, Error)]
#[error("Path not found: {0}")]
pub struct PathDenied(pub String);

/// Method not allowed error type for middleware
///
/// Returns HTTP 405 Method Not Allowed when a request uses a disallowed HTTP method.
#[derive(Debug, Error)]
#[error("Method not allowed: {0}")]
pub struct MethodDenied(pub String);

/// Unsupported media type error for middleware
///
/// Returns HTTP 415 Unsupported Media Type when content-type is not allowed.
#[derive(Debug, Error)]
#[error("Unsupported media type: {0}")]
pub struct ContentTypeDenied(pub String);

/// Rate limit exceeded error for middleware
///
/// Returns HTTP 429 Too Many Requests when rate limit is exceeded.
#[derive(Debug, Error)]
#[error("Rate limit exceeded: {0}")]
pub struct RateLimitExceeded(pub String);

/// Access forbidden error for middleware
///
/// Returns HTTP 403 Forbidden for IP/CIDR-based access control denials.
#[derive(Debug, Error)]
#[error("Access forbidden: {0}")]
pub struct AccessDenied(pub String);

/// Upstream failure error for middleware
///
/// A backend reported that the requested operation failed, for example a
/// DIMSE C-FIND, C-GET or C-MOVE that could not reach the remote PACS.
/// Maps to HTTP 502 Bad Gateway and DIMSE status 0xA701.
///
/// The message names the operation and carries the raw backend error, which
/// often includes hosts, ports and AE titles. It is for server logs only.
/// Adapters must never send it to clients.
#[derive(Debug, Error)]
#[error("Upstream {operation} failed: {detail}")]
pub struct UpstreamFailure {
    pub operation: String,
    pub detail: String,
}
