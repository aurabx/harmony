use crate::config::config::Config;
use crate::models::connection::ConnectionConfig;
use std::collections::HashMap;

/// Resolves all references in the configuration
pub fn resolve_references(config: &mut Config) -> Result<(), String> {
    resolve_endpoints(config)?;
    resolve_backends(config)?;
    Ok(())
}

/// Resolves an authentication reference to the actual AuthenticationDefinition
fn resolve_authentication_ref(
    auth_ref: &str,
    authentications: &HashMap<String, crate::models::connection::AuthenticationDefinition>,
) -> Result<crate::models::connection::AuthenticationDefinition, String> {
    authentications
        .get(auth_ref)
        .cloned()
        .ok_or_else(|| format!("Authentication '{}' not found in global authentications", auth_ref))
}

fn resolve_endpoints(config: &mut Config) -> Result<(), String> {
    let peers = config.peers.clone();
    
    for (name, endpoint) in config.endpoints.iter_mut() {
        if let Some(peer_ref) = &endpoint.peer_ref {
            let peer = peers.get(peer_ref).ok_or_else(|| {
                format!("Endpoint '{}' references non-existent peer '{}'", name, peer_ref)
            })?;

            if !peer.enabled {
                 tracing::warn!("Endpoint '{}' references disabled peer '{}'", name, peer_ref);
            }

            // Merge connection settings
            let mut resolved_connection = peer.connection.clone();
            
            // Peer protocol (formerly type)
            if let Some(protocol) = peer.get_protocol() {
                if resolved_connection.protocol.is_none() {
                    resolved_connection.protocol = Some(protocol);
                }
            }

            // Override with endpoint connection settings if present
            if let Some(endpoint_conn) = &endpoint.connection {
                 if !endpoint_conn.host.is_empty() {
                     resolved_connection.host = endpoint_conn.host.clone();
                 }
                 if endpoint_conn.port.is_some() {
                     resolved_connection.port = endpoint_conn.port;
                 }
                 if endpoint_conn.protocol.is_some() {
                     resolved_connection.protocol = endpoint_conn.protocol.clone();
                 }
                 if endpoint_conn.base_path.is_some() {
                     resolved_connection.base_path = endpoint_conn.base_path.clone();
                 }
            }
            
            endpoint.connection = Some(resolved_connection);
            
            // Merge authentication
            if endpoint.authentication.is_none() {
                endpoint.authentication = peer.authentication.clone();
            }
        }

        // Inject connection into options for services
        inject_connection_into_options(&mut endpoint.options, &endpoint.connection);
        // Authentication is now a reference string - resolved during middleware construction
    }
    Ok(())
}

fn resolve_backends(config: &mut Config) -> Result<(), String> {
    let targets = config.targets.clone();

    for (name, backend) in config.backends.iter_mut() {
        if let Some(target_ref) = &backend.target_ref {
            let target = targets.get(target_ref).ok_or_else(|| {
                format!("Backend '{}' references non-existent target '{}'", name, target_ref)
            })?;

            if !target.enabled {
                tracing::warn!("Backend '{}' references disabled target '{}'", name, target_ref);
            }

            // Merge connection settings
            let mut resolved_connection = target.connection.clone();

             // Target protocol (formerly type)
            if let Some(protocol) = target.get_protocol() {
                if resolved_connection.protocol.is_none() {
                    resolved_connection.protocol = Some(protocol);
                }
            }

            // Override with backend connection settings
            if let Some(backend_conn) = &backend.connection {
                 if !backend_conn.host.is_empty() {
                     resolved_connection.host = backend_conn.host.clone();
                 }
                 if backend_conn.port.is_some() {
                     resolved_connection.port = backend_conn.port;
                 }
                 if backend_conn.protocol.is_some() {
                     resolved_connection.protocol = backend_conn.protocol.clone();
                 }
                 if backend_conn.base_path.is_some() {
                     resolved_connection.base_path = backend_conn.base_path.clone();
                 }
            }

            backend.connection = Some(resolved_connection);

            // Merge authentication
            if backend.authentication.is_none() {
                backend.authentication = target.authentication.clone();
            }

            // Merge reliability
            if backend.timeout_secs.is_none() {
                backend.timeout_secs = Some(target.timeout_secs);
            }
            if backend.max_retries.is_none() {
                backend.max_retries = Some(target.max_retries);
            }
        }
        
        // Inject connection into options for services
        inject_connection_into_options(&mut backend.options, &backend.connection);
        
        // Resolve and inject authentication definition into options
        if let Some(auth_ref) = &backend.authentication {
            match resolve_authentication_ref(auth_ref, &config.authentications) {
                Ok(auth_def) => {
                    let options_map = backend.options.get_or_insert_with(HashMap::new);
                    options_map.insert(
                        "authentication_def".to_string(),
                        serde_json::to_value(&auth_def).expect("Failed to serialize AuthenticationDefinition"),
                    );
                }
                Err(e) => {
                    tracing::warn!("Backend '{}': {}", name, e);
                }
            }
        }
        
        // Inject reliability
        if let Some(options) = &mut backend.options {
            if let Some(timeout) = backend.timeout_secs {
                if !options.contains_key("timeout_secs") {
                    options.insert("timeout_secs".to_string(), serde_json::json!(timeout));
                }
            }
             if let Some(retries) = backend.max_retries {
                if !options.contains_key("max_retries") {
                    options.insert("max_retries".to_string(), serde_json::json!(retries));
                }
            }
        }
    }
    Ok(())
}

fn inject_connection_into_options(
    options: &mut Option<HashMap<String, serde_json::Value>>,
    connection: &Option<ConnectionConfig>
) {
    if let Some(conn) = connection {
        let options_map = options.get_or_insert_with(HashMap::new);
        
        // Inject individual fields if not present (to support legacy lookups if we updated services)
        // But more importantly, inject the whole connection object
        options_map.insert("connection".to_string(), serde_json::to_value(conn).unwrap());
        
        // Also polyfill common fields for backward compatibility if we want,
        // but strictly speaking we should update services to look at "connection" object.
        // For now, let's just inject the object.
    }
}

// inject_auth_into_options removed - authentication is now a reference string
// resolved during middleware construction phase
