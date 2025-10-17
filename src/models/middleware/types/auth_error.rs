use thiserror::Error;

/// Authentication failure error type for middleware
///
/// Carry a message for compatibility with existing tests and logs.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct AuthFailure(pub &'static str);
