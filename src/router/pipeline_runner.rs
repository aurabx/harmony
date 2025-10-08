use crate::config::config::Config;
use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::chain::MiddlewareChain;
use crate::models::pipelines::config::Pipeline;
use std::collections::HashMap;
use std::sync::Arc;

pub async fn run_pipeline(
    envelope: RequestEnvelope<Vec<u8>>,
    pipeline_name: &str,
    config: Arc<Config>,
) -> Result<RequestEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    let group: &Pipeline = config
        .pipelines
        .get(pipeline_name)
        .ok_or_else(|| format!("Unknown pipeline '{}'", pipeline_name))?;

    // 1. Incoming (left) middleware chain
    let after_incoming_mw = process_incoming_middleware(envelope, group, &config).await?;

    // 2. Backends
    let after_backends = process_backends(after_incoming_mw, group, &config).await?;

    // 3. Outgoing (right) middleware chain
    let after_outgoing_mw = process_outgoing_middleware(after_backends, group, &config).await?;

    Ok(after_outgoing_mw)
}

async fn process_backends(
    mut envelope: RequestEnvelope<Vec<u8>>,
    group: &Pipeline,
    config: &Config,
) -> Result<RequestEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    for backend_name in &group.backends {
        if let Some(backend) = config.backends.get(backend_name) {
            let service = backend
                .resolve_service()
                .map_err(|err| format!("Failed to resolve backend service: {}", err))?;

            envelope = service
                .transform_request(
                    envelope,
                    backend.options.as_ref().unwrap_or(&HashMap::new()),
                )
                .await
                .map_err(|err| format!("Backend request transformation failed: {:?}", err))?;
        } else {
            tracing::warn!("Backend '{}' not found in config", backend_name);
        }
    }
    Ok(envelope)
}

async fn process_incoming_middleware(
    envelope: RequestEnvelope<Vec<u8>>,
    group: &Pipeline,
    config: &Config,
) -> Result<RequestEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    // Clone normalized_data before using it to avoid ownership issues
    let normalized_data = envelope.normalized_data.clone();

    // Convert envelope to use serde_json::Value for middleware processing
    let json_envelope = RequestEnvelope {
        request_details: envelope.request_details.clone(),
        original_data: normalized_data.unwrap_or_else(|| {
            serde_json::from_slice(&envelope.original_data).unwrap_or(serde_json::Value::Null)
        }),
        normalized_data: envelope.normalized_data.clone(),
    };

    // Build middleware instances from pipeline names + config options
    let middleware_instances: Vec<(String, HashMap<String, serde_json::Value>)> = group
        .middleware
        .iter()
        .map(|raw| resolve_middleware_instance(raw, config))
        .collect();

    let middleware_chain = MiddlewareChain::new(&middleware_instances);

    // Process through middleware chain
    let processed_json_envelope = middleware_chain.left(json_envelope).await?;

    // Convert back to Vec<u8> envelope
    let processed_envelope = RequestEnvelope {
        request_details: processed_json_envelope.request_details,
        original_data: envelope.original_data, // Keep original bytes
        normalized_data: processed_json_envelope.normalized_data,
    };

    Ok(processed_envelope)
}

async fn process_outgoing_middleware(
    envelope: RequestEnvelope<Vec<u8>>,
    group: &Pipeline,
    config: &Config,
) -> Result<RequestEnvelope<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    // Clone normalized_data before using it to avoid ownership issues
    let normalized_data = envelope.normalized_data.clone();

    // Convert envelope to use serde_json::Value for middleware processing
    let json_envelope = RequestEnvelope {
        request_details: envelope.request_details.clone(),
        original_data: normalized_data.unwrap_or_else(|| {
            serde_json::from_slice(&envelope.original_data).unwrap_or(serde_json::Value::Null)
        }),
        normalized_data: envelope.normalized_data.clone(),
    };

    // Build middleware instances from pipeline names + config options
    let middleware_instances: Vec<(String, HashMap<String, serde_json::Value>)> = group
        .middleware
        .iter()
        .map(|raw| resolve_middleware_instance(raw, config))
        .collect();

    let middleware_chain = MiddlewareChain::new(&middleware_instances);

    // Process through middleware chain (right side)
    let processed_json_envelope = middleware_chain.right(json_envelope).await?;

    // Convert back to Vec<u8> envelope
    let processed_envelope = RequestEnvelope {
        request_details: processed_json_envelope.request_details,
        original_data: envelope.original_data, // Keep original bytes
        normalized_data: Some(processed_json_envelope.original_data),
    };

    Ok(processed_envelope)
}

// @todo Should be in middleware
fn resolve_middleware_instance(
    raw_name: &str,
    config: &Config,
) -> (String, HashMap<String, serde_json::Value>) {
    // Normalize name: accept forms like "middleware.jwt_auth" or "jwt_auth"
    let base = raw_name
        .split('.')
        .next_back()
        .unwrap_or(raw_name)
        .to_lowercase();

    // Helper to turn a config struct into a map
    let to_map = |val: serde_json::Value| -> HashMap<String, serde_json::Value> {
        match val {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            _ => HashMap::new(),
        }
    };

    match base.as_str() {
        // JWT auth
        "jwt_auth" | "jwtauth" => {
            let opts = config
                .middleware
                .jwt_auth
                .as_ref()
                .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null))
                .map(to_map)
                .unwrap_or_default();
            ("jwtauth".to_string(), opts)
        }
        // Basic/Auth sidecar
        "basic_auth" | "auth_sidecar" | "auth" => {
            let opts = config
                .middleware
                .auth_sidecar
                .as_ref()
                .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null))
                .map(to_map)
                .unwrap_or_default();
            ("auth".to_string(), opts)
        }
        // Aurabox connect
        "aurabox_connect" | "connect" => {
            let opts = config
                .middleware
                .aurabox_connect
                .as_ref()
                .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null))
                .map(to_map)
                .unwrap_or_default();
            ("connect".to_string(), opts)
        }
        // Fallback: pass the normalized name with no options
        other => (other.to_string(), HashMap::new()),
    }
}
