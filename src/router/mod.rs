use axum::{
    Router,
};
// use axum::middleware::from_fn_with_state;
// use tower::ServiceBuilder;
use crate::config::{Config};
use crate::middleware::build_middleware_stack;

use crate::endpoints::basic::BasicEndpointHandler;
use crate::endpoints::{EndpointHandler, EndpointHandlerFactory};
use crate::groups::config::Group;

/**
 * Builds the router for the application
 */
pub async fn build_network_router(config: &Config, network_name: &str) -> Router {
    let mut app = Router::new();

    // Process only groups that are configured for this network
    for (group_name, group) in &config.groups {
        if !group.networks.contains(&network_name.to_string()) {
            continue; // Skip groups not configured for this network
        }

        // Process incoming endpoints if present
        if !group.endpoints.is_empty() {
            app = build_incoming_routes(app, group_name, group, config);
        }

        // Process outgoing backends if present
        if !group.backends.is_empty() {
            app = build_outgoing_routes(app, group_name, group, config);
        }
    }

    app
}


/**
 * Builds the router for the incoming endpoints
 */
fn build_incoming_routes(
    mut app: Router,
    group_name: &str,
    group: &Group,
    config: &Config,
) -> Router {
    let mut factory = EndpointHandlerFactory::new();
    let mut processed_endpoints = std::collections::HashSet::new();

    for endpoint_name in &group.endpoints {
        if !processed_endpoints.insert(endpoint_name) {
            continue;
        }

        let endpoint = match config.endpoints.get(endpoint_name) {
            Some(e) => e,
            None => {
                tracing::error!("Endpoint '{}' referenced in group '{}' not found", endpoint_name, group_name);
                continue;
            }
        };

        let handler = match factory.create_handler(&endpoint.kind) {
            Ok(handler) => handler,
            Err(e) => {
                tracing::error!("Failed to create handler for endpoint '{}': {}", endpoint_name, e);
                Box::new(BasicEndpointHandler)
            }
        };

        let mut route = handler.create_router();

        // Add incoming middleware using the generic middleware stack
        if !group.middleware.incoming.is_empty() {
            route = build_middleware_stack(route, &group.middleware.incoming, config.middleware.clone());
        }

        app = app.nest(&endpoint.path_prefix, route);
    }

    app
}

/**
 * Builds the router for the outgoing backends
 */
fn build_outgoing_routes(
    mut app: Router,
    group_name: &str,
    group: &Group,
    config: &Config,
) -> Router {
    let mut processed_backends = std::collections::HashSet::new();

    for backend_name in &group.backends {
        if !processed_backends.insert(backend_name) {
            continue;
        }

        let _backend = match config.backends.get(backend_name) {
            Some(b) => b,
            None => {
                tracing::error!("Backend '{}' referenced in group '{}' not found", backend_name, group_name);
                continue;
            }
        };

        // Create a basic handler for the backend
        let handler = Box::new(BasicEndpointHandler);
        let mut route = handler.create_router();

        // Add outgoing middleware
        if !group.middleware.outgoing.is_empty() {
            route = build_middleware_stack(route, &group.middleware.outgoing, config.middleware.clone());
        }

        // For outgoing routes, we'll use the backend name as the path prefix
        // You might want to adjust this based on your specific needs
        let path_prefix = format!("/{}", backend_name);
        app = app.nest(&path_prefix, route);
    }

    app
}

// Update endpoint and backend validation to check for required DICOM ports


