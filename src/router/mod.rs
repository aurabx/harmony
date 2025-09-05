use axum::{
    Router,
    body::Body,
    http::Request,
    routing::any,
    extract::State,
};
// use axum::middleware::from_fn_with_state;
// use tower::ServiceBuilder;
use crate::config::{Config};
use crate::middleware::build_middleware_stack;

use crate::endpoints::basic::BasicEndpointHandler;
use crate::endpoints::{EndpointHandlerFactory};
use crate::groups::config::Group;
use crate::backends::config::{BackendType};
use http::StatusCode;
use axum::body::to_bytes;

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
 * Selects the appropriate backend configuration for an endpoint within a group.
 * Handles all backend types (e.g., DICOM, FHIR, PassThru).
 */
#[inline]
fn select_backend_for_endpoint<'a>(
    group: &'a Group,
    config: &'a Config,
    _endpoint: &crate::endpoints::config::Endpoint,
) -> Option<&'a crate::backends::config::Backend> {
    // If the group has exactly one backend, prefer it
    if group.backends.len() == 1 {
        let backend_name = &group.backends[0];
        return config.backends.get(backend_name);
    }

    // If multiple backends, iterate in group order and return the first valid backend configuration
    for backend_name in &group.backends {
        if let Some(backend_cfg) = config.backends.get(backend_name) {
            return Some(backend_cfg);
        }
    }

    None // No suitable backend found
}

#[derive(Clone)]
enum ProxyState {
    Http {
        base_url: String,
        client: reqwest::Client,
    },
    Dicom {
        ip: String,
        port: u16,
        aet: String,
    },
}

async fn proxy_handler(State(state): State<ProxyState>, req: Request<Body>) -> axum::response::Response {
    match state {
        ProxyState::Http { base_url, client } => {
            // Handle HTTP-based proxying (e.g., FHIR)
            let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
            let target = format!("{}{}", base_url.trim_end_matches('/'), path_and_query);

            let method = req.method().clone();
            let headers_clone = req.headers().clone();

            let (_parts, body) = req.into_parts();
            let body_bytes = match to_bytes(body, usize::MAX).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("Failed to read request body: {}", e);
                    return axum::response::Response::builder()
                        .status(StatusCode::BAD_GATEWAY)
                        .body(Body::from("Failed to read request body"))
                        .unwrap();
                }
            };

            let mut builder = client.request(method, &target);
            for (k, v) in headers_clone.iter() {
                if k.as_str().eq_ignore_ascii_case("host") {
                    continue;
                }
                builder = builder.header(k, v);
            }

            let result = builder.body(body_bytes).send().await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    let headers = resp.headers().clone();
                    let bytes = resp.bytes().await.unwrap_or_else(|e| {
                        tracing::error!("Failed to read backend response body: {}", e);
                        bytes::Bytes::new()
                    });
                    let mut response = axum::response::Response::builder().status(status);
                    {
                        let headers_mut = response.headers_mut().unwrap();
                        for (k, v) in headers.iter() {
                            headers_mut.append(k, v.clone());
                        }
                    }
                    response.body(Body::from(bytes)).unwrap()
                }
                Err(_e) => axum::response::Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from("Backend request failed"))
                    .unwrap(),
            }
        }
        ProxyState::Dicom { ip, port, aet } => {
            // Handle DICOM-based proxying here
            tracing::info!(
                "DICOM proxying not yet implemented. IP: {}, Port: {}, AET: {}",
                ip,
                port,
                aet
            );
            axum::response::Response::builder()
                .status(StatusCode::NOT_IMPLEMENTED)
                .body(Body::from("DICOM proxying is not yet implemented"))
                .unwrap()
        }
    }
}

fn build_proxy_router(backend: &crate::backends::config::Backend) -> Router {
    match &backend.type_ {
        BackendType::Fhir { url } => {
            let state = ProxyState::Http {
                base_url: url.clone(),
                client: reqwest::Client::new(),
            };
            Router::new().route("/*path", any(proxy_handler)).with_state(state)
        }
        BackendType::Dicom { host, port, aet } => {
            let state = ProxyState::Dicom {
                ip: host.clone(),
                port: *port,
                aet: aet.clone(),
            };
            Router::new().route("/*path", any(proxy_handler)).with_state(state)
        }
        BackendType::PassThru | BackendType::DeadLetter => {
            tracing::warn!("Unsupported backend type for proxying: {:?}", backend.type_);
            Router::new().route("/", any(|| async { "Unsupported backend type" }))
        }
    }
}

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
                tracing::error!(
                    "Endpoint '{}' referenced in group '{}' not found",
                    endpoint_name,
                    group_name
                );
                continue;
            }
        };

        // Determine backend configuration for this endpoint/group
        let mut route = if let Some(backend) = select_backend_for_endpoint(group, config, endpoint)
        {
            build_proxy_router(backend)
        } else {
            let handler = match factory.create_handler(&endpoint.kind) {
                Ok(handler) => handler,
                Err(e) => {
                    tracing::error!(
                        "Failed to create handler for endpoint '{}': {}",
                        endpoint_name,
                        e
                    );
                    Box::new(BasicEndpointHandler)
                }
            };
            handler.create_router()
        };

        // Add incoming middleware using the generic middleware stack
        if !group.middleware.incoming.is_empty() {
            route =
                build_middleware_stack(route, &group.middleware.incoming, config.middleware.clone());
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

        let backend = match config.backends.get(backend_name) {
            Some(b) => b,
            None => {
                tracing::error!(
                    "Backend '{}' referenced in group '{}' not found",
                    backend_name,
                    group_name
                );
                continue;
            }
        };

        let mut route = build_proxy_router(backend);

        // Add outgoing middleware
        if !group.middleware.outgoing.is_empty() {
            route =
                build_middleware_stack(route, &group.middleware.outgoing, config.middleware.clone());
        }

        let path_prefix = format!("/{}", backend_name);
        app = app.nest(&path_prefix, route);
    }

    app
}

// Update endpoint and backend validation to check for required DICOM ports
