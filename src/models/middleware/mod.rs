pub mod chain;
pub(crate) mod instance;
#[allow(clippy::module_inception)]
pub mod middleware;
pub mod types;

// Re-export error types for easier access
pub use types::auth_error::AuthFailure;
pub use types::denial_errors::{
    AccessDenied, ContentTypeDenied, MethodDenied, PathDenied, RateLimitExceeded,
};

use axum::response::Response;
use http::Request;
use serde::de::StdError;
use tower::util::BoxCloneService;

pub type Next<B> = BoxCloneService<Request<B>, Response, Box<dyn StdError + Send + Sync>>;
