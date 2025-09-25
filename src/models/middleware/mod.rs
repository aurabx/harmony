pub mod chain;
pub mod config;
pub mod middleware;
pub mod types;


use axum::response::Response;
use http::Request;
use serde::de::StdError;
use tower::util::BoxCloneService;
use crate::models::middleware::config::*;

pub struct MiddlewareState {
    #[allow(dead_code)]
    config: MiddlewareConfig,
}

pub type Next<B> = BoxCloneService<Request<B>, Response, Box<dyn StdError + Send + Sync>>;
