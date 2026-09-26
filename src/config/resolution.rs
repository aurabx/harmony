use crate::config::config::Config;
use crate::models::connection::{AuthenticationDefinition, ConnectionConfig};
use std::collections::HashMap;

/// Prefix of the TOML table that holds authentication definitions. References
/// must not include it: `authentication = "fhir_bearer"`, not
/// `authentication = "authentications.fhir_bearer"`.
const AUTHENTICATIONS_TABLE_PREFIX: &str = "authentications.";

/// Resolves all references in the configuration
/// Returns a set of backend names that could not be resolved
///
/// Fails if any `authentication` reference does not name a key in
/// `[authentications]`. A missing credential must stop the load, because
/// otherwise Harmony sends requests upstream without it.
pub fn resolve_references(config: &mut Config) -> Result<std::collections::HashSet<String>, String> {
    validate_authentication_refs(config)?;
    resolve_endpoints(config)?;
    let unresolved_backends = resolve_backends(config)?;
    Ok(unresolved_backends)
}

/// Checks every `authentication` reference written on a target, peer, backend,
/// endpoint or middleware instance.
///
/// This runs before targets and peers are merged into backends and endpoints,
/// so each error names the item where the reference is written. All errors are
/// returned together, sorted, so one load reports every bad reference.
fn validate_authentication_refs(config: &Config) -> Result<(), String> {
    let mut references: Vec<(&str, &str, &str)> = Vec::new();
    references.extend(config.targets.iter().filter_map(|(name, target)| {
        target.authentication.as_deref().map(|auth_ref| ("Target", name.as_str(), auth_ref))
    }));
    references.extend(config.peers.iter().filter_map(|(name, peer)| {
        peer.authentication.as_deref().map(|auth_ref| ("Peer", name.as_str(), auth_ref))
    }));
    references.extend(config.backends.iter().filter_map(|(name, backend)| {
        backend.authentication.as_deref().map(|auth_ref| ("Backend", name.as_str(), auth_ref))
    }));
    references.extend(config.endpoints.iter().filter_map(|(name, endpoint)| {
        endpoint.authentication.as_deref().map(|auth_ref| ("Endpoint", name.as_str(), auth_ref))
    }));
    references.extend(config.middleware.iter().filter_map(|(name, middleware)| {
        middleware.authentication.as_deref().map(|auth_ref| ("Middleware", name.as_str(), auth_ref))
    }));

    let mut errors: Vec<String> = references
        .into_iter()
        .filter_map(|(kind, name, auth_ref)| {
            resolve_authentication_ref(kind, name, auth_ref, &config.authentications).err()
        })
        .collect();

    if errors.is_empty() {
        return Ok(());
    }
    errors.sort();
    Err(errors.join("\n"))
}

/// Resolves an authentication reference to the actual AuthenticationDefinition.
///
/// `kind` and `name` identify the item that holds the reference (for example
/// `"Target"` and `"fhir_server"`). They only shape the error message.
fn resolve_authentication_ref(
    kind: &str,
    name: &str,
    auth_ref: &str,
    authentications: &HashMap<String, AuthenticationDefinition>,
) -> Result<AuthenticationDefinition, String> {
    authentications
        .get(auth_ref)
        .cloned()
        .ok_or_else(|| unknown_authentication_message(kind, name, auth_ref, authentications))
}

fn unknown_authentication_message(
    kind: &str,
    name: &str,
    auth_ref: &str,
    authentications: &HashMap<String, AuthenticationDefinition>,
) -> String {
    let mut available: Vec<&str> = authentications.keys().map(String::as_str).collect();
    available.sort_unstable();
    let available = if available.is_empty() {
        "(none defined)".to_string()
    } else {
        available.join(", ")
    };

    match auth_ref.strip_prefix(AUTHENTICATIONS_TABLE_PREFIX) {
        Some(bare_key) => format!(
            "{} '{}' references unknown authentication '{}'. Drop the \"{}\" prefix: use \"{}\" instead of \"{}\". Available authentications: {}",
            kind, name, auth_ref, AUTHENTICATIONS_TABLE_PREFIX, bare_key, auth_ref, available
        ),
        None => format!(
            "{} '{}' references unknown authentication '{}'. Available authentications: {}",
            kind, name, auth_ref, available
        ),
    }
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

fn resolve_backends(config: &mut Config) -> Result<std::collections::HashSet<String>, String> {
    let targets = config.targets.clone();
    let mut unresolved_backends = std::collections::HashSet::new();

    for (name, backend) in config.backends.iter_mut() {
        if let Some(target_ref) = &backend.target_ref {
            let target = match targets.get(target_ref) {
                Some(t) => t,
                None => {
                    tracing::warn!("Backend '{}' references non-existent target '{}' - backend will be skipped", name, target_ref);
                    unresolved_backends.insert(name.clone());
                    continue;
                }
            };

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
        
        // Resolve and inject authentication definition into options.
        // validate_authentication_refs has already checked this reference; the
        // error path stays so a missing credential can never be skipped.
        if let Some(auth_ref) = &backend.authentication {
            let auth_def =
                resolve_authentication_ref("Backend", name, auth_ref, &config.authentications)?;
            let options_map = backend.options.get_or_insert_with(HashMap::new);
            options_map.insert(
                "authentication_def".to_string(),
                serde_json::to_value(&auth_def).expect("Failed to serialize AuthenticationDefinition"),
            );
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
    Ok(unresolved_backends)
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
