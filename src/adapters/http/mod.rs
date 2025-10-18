use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::models::envelope::envelope::ResponseEnvelope;
use crate::models::protocol::{Protocol, ProtocolCtx};
use crate::utils::Error;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub mod router;

/// HTTP Protocol Adapter
/// 
/// Wraps Axum HTTP server and provides protocol-specific I/O handling
/// while using the common PipelineExecutor for business logic.
pub struct HttpAdapter {
    pub network_name: String,
    pub bind_addr: SocketAddr,
}

impl HttpAdapter {
    pub fn new(network_name: String, bind_addr: SocketAddr) -> Self {
        Self {
            network_name,
            bind_addr,
        }
    }

    /// Convert Axum HTTP Request to ProtocolCtx
    pub async fn http_request_to_protocol_ctx(
        req: &mut Request,
        options: &HashMap<String, serde_json::Value>,
    ) -> Result<ProtocolCtx, Error> {
        // Compute subpath using path_prefix option
        let path_prefix = options
            .get("path_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let path_only = req.uri().path().to_string();
        let full_path_with_query = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| path_only.clone());
        let mut subpath = path_only
            .strip_prefix(path_prefix)
            .unwrap_or("")
            .to_string();
        if subpath.starts_with('/') {
            subpath = subpath.trim_start_matches('/').to_string();
        }

        // Headers
        let headers_obj: serde_json::Value = {
            let map: serde_json::Map<String, serde_json::Value> = req
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        serde_json::Value::String(v.to_str().unwrap_or_default().to_string()),
                    )
                })
                .collect();
            serde_json::Value::Object(map)
        };

        // Cookies
        let cookies_obj: serde_json::Value = {
            let mut map = serde_json::Map::new();
            for val in req.headers().get_all(http::header::COOKIE).iter() {
                if let Ok(s) = val.to_str() {
                    for part in s.split(';') {
                        let kv = part.trim();
                        if kv.is_empty() {
                            continue;
                        }
                        let mut split = kv.splitn(2, '=');
                        let name = split.next().unwrap_or("").trim();
                        let value = split.next().unwrap_or("").trim();
                        if !name.is_empty() {
                            map.insert(
                                name.to_string(),
                                serde_json::Value::String(value.to_string()),
                            );
                        }
                    }
                }
            }
            serde_json::Value::Object(map)
        };

        // Query params
        let query_obj: serde_json::Value = {
            let mut root = serde_json::Map::new();
            if let Some(q) = req.uri().query() {
                for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
                    root.entry(k.to_string())
                        .or_insert_with(|| serde_json::Value::Array(vec![]))
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::Value::String(v.to_string()));
                }
            }
            serde_json::Value::Object(root)
        };

        // Cache status
        let cache_status = req
            .headers()
            .get("Cache-Status")
            .or_else(|| req.headers().get("X-Cache"))
            .or_else(|| req.headers().get("CF-Cache-Status"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        // Metadata
        let mut meta_map = HashMap::new();
        meta_map.insert("protocol".to_string(), "http".to_string());
        meta_map.insert("path".to_string(), subpath);
        meta_map.insert("full_path".to_string(), full_path_with_query);

        // attrs object
        let mut attrs = serde_json::Map::new();
        attrs.insert(
            "method".to_string(),
            serde_json::Value::String(req.method().to_string()),
        );
        attrs.insert(
            "uri".to_string(),
            serde_json::Value::String(req.uri().to_string()),
        );
        attrs.insert("headers".to_string(), headers_obj);
        attrs.insert("cookies".to_string(), cookies_obj);
        attrs.insert("query_params".to_string(), query_obj);
        attrs.insert(
            "cache_status".to_string(),
            serde_json::Value::String(cache_status),
        );

        // Body bytes
        let body_bytes =
            axum::body::to_bytes(std::mem::replace(req.body_mut(), Body::empty()), usize::MAX)
                .await
                .map_err(|_| Error::from("Failed to read request body"))?
                .to_vec();

        Ok(ProtocolCtx {
            protocol: Protocol::Http,
            payload: body_bytes,
            meta: meta_map,
            attrs: serde_json::Value::Object(attrs),
        })
    }

    /// Convert ResponseEnvelope to Axum HTTP Response
    pub fn response_envelope_to_http(
        envelope: ResponseEnvelope<Vec<u8>>,
    ) -> Result<Response<Body>, Error> {
        let mut builder = Response::builder().status(envelope.response_details.status);

        // Add headers
        for (key, value) in &envelope.response_details.headers {
            builder = builder.header(key, value);
        }

        // Build response with body
        builder
            .body(Body::from(envelope.original_data))
            .map_err(|e| Error::from(format!("Failed to build HTTP response: {}", e)))
    }
}

#[async_trait]
impl ProtocolAdapter for HttpAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::Http
    }

    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let bind_addr = self.bind_addr;
        let network_name = self.network_name.clone();

        // Build the router using the router module
        let app = router::build_network_router(config.clone(), &network_name).await;

        Ok(tokio::spawn(async move {
            let listener = match TcpListener::bind(bind_addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!(
                        "Failed to bind HTTP adapter for network '{}' to {}: {}",
                        network_name,
                        bind_addr,
                        e
                    );
                    return;
                }
            };

            tracing::info!(
                "🚀 HTTP adapter started for network '{}' on {}",
                network_name,
                bind_addr
            );

            // Create a future for graceful shutdown
            let graceful_shutdown = async move {
                shutdown.cancelled().await;
            };

            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(graceful_shutdown)
                .await
            {
                tracing::error!(
                    "HTTP adapter for network '{}' encountered error: {}",
                    network_name,
                    e
                );
            }

            tracing::info!("HTTP adapter for network '{}' shut down", network_name);
        }))
    }

    fn summary(&self) -> String {
        format!(
            "HttpAdapter(network={}, bind={})",
            self.network_name, self.bind_addr
        )
    }
}
