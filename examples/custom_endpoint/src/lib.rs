
use axum::{Router, routing::get, Json};
use harmony::endpoints::{EndpointHandler};
use harmony::endpoints::custom::{CustomEndpointFactory};

pub struct MyCustomEndpoint;

async fn handle_custom_request() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "message": "Custom endpoint responding"
    }))
}

#[async_trait::async_trait]
impl EndpointHandler for MyCustomEndpoint {
    fn create_router(&self) -> Router {
        Router::new()
            .route("/", get(handle_custom_request))
            .route("/*path", get(handle_custom_request))
    }
}

pub struct MyCustomEndpointFactory;

impl CustomEndpointFactory for MyCustomEndpointFactory {
    fn name(&self) -> &'static str {
        "my_custom"
    }

    fn create_handler(&self) -> Box<dyn EndpointHandler> {
        Box::new(MyCustomEndpoint)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[no_mangle]
pub fn create_endpoint_factory() -> Box<dyn CustomEndpointFactory> {
    Box::new(MyCustomEndpointFactory)
}

fn main() {}