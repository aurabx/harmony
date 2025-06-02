use axum::{
    routing::{get},  // Add post to imports
    Router,
    Json,
};

use crate::endpoints::EndpointHandler;

pub struct BasicEndpointHandler;

#[async_trait::async_trait]
impl EndpointHandler for BasicEndpointHandler {
    fn create_router(&self) -> Router {
        Router::new()
            .route("/", get(handle_basic_request).post(handle_basic_request))
            .route("/*path", get(handle_basic_request).post(handle_basic_request))
    }
}

async fn handle_basic_request() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "message": "Basic endpoint responding"
    }))
}