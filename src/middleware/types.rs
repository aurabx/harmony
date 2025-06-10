use axum::{response::Response, http::Request};
use tower::util::BoxCloneService;
use std::error::Error as StdError;

pub type Next<B> = BoxCloneService<Request<B>, Response, Box<dyn StdError + Send + Sync>>;
pub type Error = Box<dyn StdError + Send + Sync>;