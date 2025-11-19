pub mod chain;
pub mod config;
pub(crate) mod instance;
#[allow(clippy::module_inception)]
pub mod middleware;
pub mod types;

// Re-export error types for easier access
pub use types::auth_error::AuthFailure;
pub use types::denial_errors::{
    AccessDenied, ContentTypeDenied, MethodDenied, PathDenied, RateLimitExceeded,
};

use crate::models::middleware::config::*;
use axum::response::Response;
use http::Request;
use serde::de::StdError;
use tower::util::BoxCloneService;

pub struct MiddlewareState {
    #[allow(dead_code)]
    config: MiddlewareConfig,
}

pub type Next<B> = BoxCloneService<Request<B>, Response, Box<dyn StdError + Send + Sync>>;
