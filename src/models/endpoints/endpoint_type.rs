use std::collections::HashMap;
use crate::models::envelope::{
    envelope::{Envelope},
};
use async_trait::async_trait;
use axum::{
    http::{Request, Response},
    Router,
};
use crate::config::config::ConfigError;
use serde_json::Value;
use crate::router::route_config::RouteConfig;

#[async_trait]
pub trait EndpointType: EndpointHandler<Value> {

    /// Validate the endpoint configuration
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError>;

    /// Returns configured routes
    fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig>;
}

#[async_trait]
pub trait EndpointHandler<T>: Send + Sync
where
    T: Send,
{
    type ReqBody: Send;
    type ResBody: Send;

    /// Handles incoming requests, producing an Envelope
    async fn handle_request(
        &self,
        envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Envelope<Vec<u8>>, crate::models::middleware::types::Error>;

    /// Handles the response stage, converting Envelope back into an HTTP response
    async fn handle_response(
        &self,
        envelope: Envelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<Response<Self::ResBody>, crate::models::middleware::types::Error>;
}

