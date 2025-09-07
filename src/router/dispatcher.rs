use std::collections::HashMap;
use std::sync::Arc;
use axum::body::Body;
use axum::{Router, response::Response, extract::State};
use axum::extract::Request;
use http::StatusCode;
use crate::config::config::Config;
use crate::models::groups::config::Group;
use crate::models::middleware::{process_request_through_chain, process_response_through_chain};
use crate::models::middleware::chain::MiddlewareChain;
use crate::models::envelope::envelope::{Envelope, RequestDetails};
use crate::models::backends::config::{Backend, BackendType};

pub struct Dispatcher<> {
    config: Arc<Config>,
}

impl<'a> Dispatcher<> {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Builds incoming routes for a specific group within the given app router.

    pub fn build_router(
        &self,
        mut app: Router<()>,
        group: &Group,
    ) -> Router<()> {
        for endpoint_name in &group.endpoints {
            if let Some(endpoint) = self.config.endpoints.get(endpoint_name) {
                let route_configs = endpoint
                    .kind
                    .build_router(endpoint.options.as_ref().unwrap_or(&HashMap::new()));

                for route_config in route_configs {
                    let path = route_config.path.clone();
                    let methods = route_config.methods.clone();

                    let mut method_router = axum::routing::MethodRouter::new();

                    for method in methods {
                        let endpoint_name = endpoint_name.clone();
                        let group_name = format!("group_{}", endpoint_name);
                        let config_ref = self.config.clone(); // <- clone Arc

                        let handler = move |mut req: Request| {
                            let endpoint_name = endpoint_name.clone();
                            let group_name = group_name.clone();
                            let config_ref = config_ref.clone();

                            async move {
                                Dispatcher::handle_request_async(
                                    &mut req,
                                    config_ref,
                                    endpoint_name,
                                    group_name,
                                )
                                    .await
                            }
                        };

                        method_router = match method {
                            http::Method::GET => method_router.get(handler),
                            http::Method::POST => method_router.post(handler),
                            http::Method::PUT => method_router.put(handler),
                            http::Method::DELETE => method_router.delete(handler),
                            http::Method::PATCH => method_router.patch(handler),
                            http::Method::HEAD => method_router.head(handler),
                            http::Method::OPTIONS => method_router.options(handler),
                            _ => method_router,
                        };
                    }

                    app = app.route(&path, method_router);
                }
            }
        }
        app
    }

    // Extract the request handling logic into a separate async function
    async fn handle_request_async(
        req: &mut Request,
        config: Arc<Config>,
        endpoint_name: String,
        group_name: String,
    ) -> Result<Response<Body>, StatusCode> {
        // Look up the endpoint and group from config
        let endpoint = config.endpoints.get(&endpoint_name)
            .ok_or(StatusCode::NOT_FOUND)?;

        let group = config.groups.iter()
            .find(|(_, g)| g.endpoints.contains(&endpoint_name))
            .map(|(_, g)| g)
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        let request_details = RequestDetails {
            method: req.method().to_string(),
            uri: req.uri().to_string(),
            headers: req
                .headers()
                .iter()
                .map(|(key, value)| {
                    (key.to_string(), value.to_str().unwrap_or_default().to_string())
                })
                .collect(),
            metadata: HashMap::new(),
        };

        // Build the envelope from the request
        let envelope = Self::build_envelope(req, request_details).await
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        // 1. Process through endpoint
        let mut processed_envelope = endpoint
            .kind
            .handle_request(envelope, endpoint.options.as_ref().unwrap_or(&HashMap::new()))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // 2. Incoming middleware
        processed_envelope = Self::process_incoming_middleware(processed_envelope, group, &*config).await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // 3. Backends
        processed_envelope = Self::process_backends(processed_envelope, group, &*config).await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // 4. Outgoing middleware
        processed_envelope = Self::process_outgoing_middleware(processed_envelope, group, &*config).await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // 5. Final endpoint response processing
        let response = endpoint
            .kind
            .handle_response(
                processed_envelope,
                endpoint.options.as_ref().unwrap_or(&HashMap::new()),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Convert the response to axum's expected format
        let (parts, body) = response.into_parts();
        let body_string = serde_json::to_string(&body)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        Ok(Response::from_parts(parts, Body::from(body_string)))
    }

    // Stub: Process through incoming middleware chain
    async fn process_incoming_middleware(
        mut envelope: Envelope<Vec<u8>>,
        group: &Group,
        config: &Config,
    ) -> Result<Envelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        // Build middleware chain from group config
        let middleware_chain = MiddlewareChain::new(&group.middleware, &config.middleware);

        // For now, we'll stub this out - in a real implementation, we'd need to:
        // 1. Convert envelope to Request<Body>
        // 2. Process through middleware chain
        // 3. Convert back to envelope

        tracing::info!("Processing incoming middleware for {} middlewares", group.middleware.len());

        // Stub: Just add a marker that middleware was processed
        if let Some(ref mut normalized_data) = envelope.normalized_data {
            if let Some(obj) = normalized_data.as_object_mut() {
                obj.insert("middleware_processed".to_string(), serde_json::Value::Bool(true));
            }
        }

        Ok(envelope)
    }

    // Stub: Process through backend(s)
    async fn process_backends(
        mut envelope: Envelope<Vec<u8>>,
        group: &Group,
        config: &Config,
    ) -> Result<Envelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Processing through {} backends", group.backends.len());

        // Process each backend in the group
        for backend_name in &group.backends {
            if let Some(backend) = config.backends.get(backend_name) {
                envelope = Self::process_single_backend(envelope, backend, config).await?;
            } else {
                tracing::warn!("Backend '{}' not found in config", backend_name);
            }
        }

        Ok(envelope)
    }

    // Stub: Process through a single backend
    async fn process_single_backend(
        mut envelope: Envelope<Vec<u8>>,
        backend: &Backend,
        config: &Config,
    ) -> Result<Envelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        match &backend.type_ {
            BackendType::Dicom { aet, host, port } => {
                tracing::info!("Processing DICOM backend: {}@{}:{}", aet, host, port);

                // Stub: Add DICOM processing marker
                if let Some(ref mut normalized_data) = envelope.normalized_data {
                    if let Some(obj) = normalized_data.as_object_mut() {
                        obj.insert("dicom_processed".to_string(), serde_json::json!({
                            "aet": aet,
                            "host": host,
                            "port": port
                        }));
                    }
                }
            },
            BackendType::Fhir { url } => {
                tracing::info!("Processing FHIR backend: {}", url);

                // Stub: Add FHIR processing marker
                if let Some(ref mut normalized_data) = envelope.normalized_data {
                    if let Some(obj) = normalized_data.as_object_mut() {
                        obj.insert("fhir_processed".to_string(), serde_json::json!({
                            "url": url
                        }));
                    }
                }
            },
            BackendType::PassThru => {
                tracing::info!("Processing PassThru backend");

                // Stub: Add passthrough marker
                if let Some(ref mut normalized_data) = envelope.normalized_data {
                    if let Some(obj) = normalized_data.as_object_mut() {
                        obj.insert("passthru_processed".to_string(), serde_json::Value::Bool(true));
                    }
                }
            },
            BackendType::DeadLetter => {
                tracing::info!("Processing DeadLetter backend");

                // Stub: Add dead letter marker
                if let Some(ref mut normalized_data) = envelope.normalized_data {
                    if let Some(obj) = normalized_data.as_object_mut() {
                        obj.insert("deadletter_processed".to_string(), serde_json::Value::Bool(true));
                    }
                }
            }
        }

        Ok(envelope)
    }

    // Stub: Process through outgoing middleware chain
    async fn process_outgoing_middleware(
        mut envelope: Envelope<Vec<u8>>,
        group: &Group,
        config: &Config,
    ) -> Result<Envelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        // In a real implementation, this would process the outgoing middleware chain
        // For now, just log and add a marker

        tracing::info!("Processing outgoing middleware for {} middlewares", group.middleware.len());

        // Stub: Just add a marker that outgoing middleware was processed
        if let Some(ref mut normalized_data) = envelope.normalized_data {
            if let Some(obj) = normalized_data.as_object_mut() {
                obj.insert("outgoing_middleware_processed".to_string(), serde_json::Value::Bool(true));
            }
        }

        Ok(envelope)
    }

    async fn build_envelope(req: &mut Request, request_details: RequestDetails) -> Result<Envelope<Vec<u8>>, StatusCode> {
        let body_bytes = axum::body::to_bytes(
            std::mem::replace(req.body_mut(), Body::empty()),
            usize::MAX
        )
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?
            .to_vec();

        let body_value: Option<serde_json::Value> =
            serde_json::from_slice(&body_bytes).ok();

        let envelope = Envelope {
            request_details,
            original_data: body_bytes,
            normalized_data: body_value,
        };

        Ok(envelope)
    }
}