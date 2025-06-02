pub mod config;

use axum::Router;
use tower::ServiceBuilder;
use crate::middleware::config::MiddlewareConfig;

pub fn build_middleware_stack(
    router: Router,
    middleware_list: &[String],
    config: MiddlewareConfig,
) -> Router {
    let service_builder = ServiceBuilder::new();

    for middleware_name in middleware_list {
        match middleware_name.as_str() {
            "jwt_auth" => {
                if let Some(jwt_config) = &config.jwt_auth {
                    // TODO: Implement JWT authentication middleware
                    tracing::info!("Adding JWT auth middleware with config: {:?}", jwt_config);
                }
            },
            "auth_sidecar" => {
                if let Some(sidecar_config) = &config.auth_sidecar {
                    // TODO: Implement auth sidecar middleware
                    tracing::info!("Adding auth sidecar middleware with config: {:?}", sidecar_config);
                }
            },
            "aurabox_connect" => {
                if let Some(aurabox_config) = &config.aurabox_connect {
                    // TODO: Implement aurabox connect middleware
                    tracing::info!("Adding aurabox connect middleware with config: {:?}", aurabox_config);
                }
            },
            _ => {
                tracing::warn!("Unknown middleware: {}", middleware_name);
            }
        }
    }

    router.layer(service_builder)
}