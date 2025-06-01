
use axum::Router;
use tower::ServiceBuilder;
use crate::config::MiddlewareConfig;

pub fn build_middleware_stack(
    router: Router,
    middleware_list: &[String],
    _config: &MiddlewareConfig,
) -> Router {
    let service_builder = ServiceBuilder::new();

    for middleware_name in middleware_list {
        match middleware_name.as_str() {
            "jwt_auth" => {
                // if let Some(jwt_config) = &config.jwt_auth {
                //     // TODO: Implement JWT authentication middleware
                //     tracing::info!("Adding JWT auth middleware");
                // }
            },
            "audit_log" => {
                // TODO: Implement audit logging middleware
                tracing::info!("Adding audit log middleware");
            },
            _ => {
                tracing::warn!("Unknown middleware: {}", middleware_name);
            }
        }
    }

    router.layer(service_builder)
}