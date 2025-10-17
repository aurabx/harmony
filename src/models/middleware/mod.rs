pub mod chain;
pub mod config;
#[allow(clippy::module_inception)]
pub mod middleware;
pub mod types;
pub(crate) mod instance;

// Re-export AuthFailure for easier access
pub use types::auth_error::AuthFailure;

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
