use crate::config::config::Config;
use crate::models::connection::{AuthenticationDefinition, ConnectionConfig};
use crate::models::services::backend_auth::{
    credential_headers, HEADER_CREDENTIAL_METHODS, JWT_METHOD, NO_CREDENTIAL_METHOD,
};
use std::borrow::Cow;
use std::collections::HashMap;

/// Prefix of the TOML table that holds authentication definitions. References
/// must not include it: `authentication = "fhir_bearer"`, not
/// `authentication = "authentications.fhir_bearer"`.
const AUTHENTICATIONS_TABLE_PREFIX: &str = "authentications.";

/// Options key that carries a resolved authentication definition to services
/// and middleware. Only reference resolution may set it.
const AUTHENTICATION_DEF_OPTION: &str = "authentication_def";

/// Middleware type that sends its credential as a request header.
const WEBHOOK_MIDDLEWARE_TYPE: &str = "webhook";

/// Middleware type whose `policies` option lists policy ids.
const POLICIES_MIDDLEWARE_TYPE: &str = "policies";

/// Resolves all references in the configuration
/// Returns a set of backend names that could not be resolved
///
/// Fails if any `authentication` reference does not name a key in
/// `[authentications]`. A missing credential must stop the load, because
/// otherwise Harmony sends requests upstream without it.
///
/// Fails if an authentication definition cannot produce its credential (a
/// missing or empty required option, or an unknown method), if a target or
/// backend references a method it cannot send, or if any options table sets
/// `authentication_def` directly. Each of these would otherwise send requests
/// upstream without a credential and without an error.
///
/// Fails if a policy lists a rule id that is not a key in `[rules]`, or a
/// `policies` middleware instance lists a policy id that is not a key in
/// `[policies]`. The middleware resolves these ids per request, so a bad id
/// that passed the load would turn every request into a 500.
pub fn resolve_references(config: &mut Config) -> Result<std::collections::HashSet<String>, String> {
    validate_no_inline_authentication_defs(config)?;
    validate_authentication_refs(config)?;
    validate_authentication_definitions(config)?;
    validate_authentication_methods_per_reference(config)?;
    validate_policy_refs(config)?;
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
    sorted_errors(
        authentication_references(config)
            .into_iter()
            .filter_map(|(kind, name, auth_ref)| {
                resolve_authentication_ref(kind, name, auth_ref, &config.authentications).err()
            })
            .collect(),
    )
}

/// Lists every `authentication` reference as `(kind, item name, reference)`,
/// where it is written in the config.
fn authentication_references(config: &Config) -> Vec<(&'static str, &str, &str)> {
    let mut references: Vec<(&'static str, &str, &str)> = Vec::new();
    references.extend(config.targets.iter().filter_map(|(name, target)| {
        target
            .authentication
            .as_deref()
            .map(|auth_ref| ("Target", name.as_str(), auth_ref))
    }));
    references.extend(config.peers.iter().filter_map(|(name, peer)| {
        peer.authentication
            .as_deref()
            .map(|auth_ref| ("Peer", name.as_str(), auth_ref))
    }));
    references.extend(config.backends.iter().filter_map(|(name, backend)| {
        backend
            .authentication
            .as_deref()
            .map(|auth_ref| ("Backend", name.as_str(), auth_ref))
    }));
    references.extend(config.endpoints.iter().filter_map(|(name, endpoint)| {
        endpoint
            .authentication
            .as_deref()
            .map(|auth_ref| ("Endpoint", name.as_str(), auth_ref))
    }));
    references.extend(config.middleware.iter().filter_map(|(name, middleware)| {
        middleware
            .authentication
            .as_deref()
            .map(|auth_ref| ("Middleware", name.as_str(), auth_ref))
    }));
    references
}

/// Joins errors into one sorted message, or returns `Ok` when there are none.
fn sorted_errors(mut errors: Vec<String>) -> Result<(), String> {
    if errors.is_empty() {
        return Ok(());
    }
    errors.sort();
    Err(errors.join("\n"))
}

/// Rejects `authentication_def` written directly in a backend, endpoint or
/// middleware options table.
///
/// Services and middleware read that key at request time. Reference
/// resolution writes it from `[authentications]` after validation, so a
/// hand-written value would skip every load-time check.
fn validate_no_inline_authentication_defs(config: &Config) -> Result<(), String> {
    let mut items: Vec<(&str, &str)> = Vec::new();
    items.extend(
        config
            .backends
            .iter()
            .filter(|(_, backend)| has_inline_definition(backend.options.as_ref()))
            .map(|(name, _)| ("Backend", name.as_str())),
    );
    items.extend(
        config
            .endpoints
            .iter()
            .filter(|(_, endpoint)| has_inline_definition(endpoint.options.as_ref()))
            .map(|(name, _)| ("Endpoint", name.as_str())),
    );
    items.extend(
        config
            .middleware
            .iter()
            .filter(|(_, middleware)| has_inline_definition(Some(&middleware.options)))
            .map(|(name, _)| ("Middleware", name.as_str())),
    );

    sorted_errors(
        items
            .into_iter()
            .map(|(kind, name)| {
                format!(
                    "{} '{}' sets '{}' in its options. Define the credential under [authentications] and set authentication = \"<key>\" instead",
                    kind,
                    escape_control_chars(name),
                    AUTHENTICATION_DEF_OPTION
                )
            })
            .collect(),
    )
}

fn has_inline_definition(options: Option<&HashMap<String, serde_json::Value>>) -> bool {
    options.is_some_and(|options| options.contains_key(AUTHENTICATION_DEF_OPTION))
}

/// Checks that every definition in `[authentications]` can produce its
/// credential, whether or not anything references it.
///
/// `jwt` definitions hold settings for the jwt_auth middleware, which checks
/// them when it is built, so only their method is checked here.
fn validate_authentication_definitions(config: &Config) -> Result<(), String> {
    sorted_errors(
        config
            .authentications
            .iter()
            .filter_map(|(key, auth_def)| definition_error(auth_def).map(|reason| (key, reason)))
            .map(|(key, reason)| {
                format!(
                    "Authentication '{}' is invalid: {}",
                    escape_control_chars(key),
                    reason
                )
            })
            .collect(),
    )
}

/// Returns why a definition is unusable, or `None` when it is usable.
fn definition_error(auth_def: &AuthenticationDefinition) -> Option<String> {
    let method = auth_def.method.as_str();
    if method == JWT_METHOD {
        return None;
    }
    let known = HEADER_CREDENTIAL_METHODS.contains(&method) || method == NO_CREDENTIAL_METHOD;
    if !known {
        return Some(format!(
            "unknown method '{}'. Supported methods: {}, {}, {}",
            escape_control_chars(method),
            HEADER_CREDENTIAL_METHODS.join(", "),
            NO_CREDENTIAL_METHOD,
            JWT_METHOD
        ));
    }
    credential_headers(auth_def).err().map(|e| e.to_string())
}

/// Checks that each reference from an item that sends the credential as a
/// request header names a definition a backend can send.
///
/// Targets, backends and webhook middleware send the credential through
/// [`credential_headers`]. A `jwt` definition is valid for jwt_auth middleware
/// but builds no request header, so one of these items that referenced it
/// would send nothing. Peer and endpoint references are not used after
/// loading, so any known method is accepted for them. Unknown references are
/// reported by `validate_authentication_refs`, which runs first.
fn validate_authentication_methods_per_reference(config: &Config) -> Result<(), String> {
    sorted_errors(
        authentication_references(config)
            .into_iter()
            .filter_map(|(kind, name, auth_ref)| {
                let sender = header_sender_noun(config, kind, name)?;
                let auth_def = config.authentications.get(auth_ref)?;
                (auth_def.method == JWT_METHOD).then(|| {
                    format!(
                        "{} '{}' references authentication '{}' with method '{}', which a {} cannot send. Use one of: {}, {}",
                        kind,
                        escape_control_chars(name),
                        escape_control_chars(auth_ref),
                        JWT_METHOD,
                        sender,
                        HEADER_CREDENTIAL_METHODS.join(", "),
                        NO_CREDENTIAL_METHOD
                    )
                })
            })
            .collect(),
    )
}

/// Returns the noun for an item that sends its credential as a request
/// header, or `None` when the item does not send one.
fn header_sender_noun(config: &Config, kind: &str, name: &str) -> Option<&'static str> {
    match kind {
        "Target" => Some("target"),
        "Backend" => Some("backend"),
        "Middleware" => config
            .middleware
            .get(name)
            .filter(|instance| {
                instance
                    .middleware_type
                    .eq_ignore_ascii_case(WEBHOOK_MIDDLEWARE_TYPE)
            })
            .map(|_| "webhook"),
        _ => None,
    }
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

/// Escapes control characters in a config-supplied string before it goes
/// into an error message.
///
/// Names and ids come from the config file, and reference errors are written
/// to stderr and the reload log. A raw newline in an id would forge a log
/// line, and a raw ESC would send an ANSI sequence to the terminal. Only
/// control characters (`char::is_control`) are escaped, so ordinary ids print
/// unchanged. A newline prints as `\n` and ESC as `\u{1b}`.
pub(crate) fn escape_control_chars(value: &str) -> Cow<'_, str> {
    if !value.chars().any(char::is_control) {
        return Cow::Borrowed(value);
    }
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
    }
    Cow::Owned(escaped)
}

/// Lists the keys of a config table, sorted, for an "unknown reference" error.
/// An empty table is spelled out so the message never ends in a bare colon.
/// Keys are escaped because TOML quoted keys can hold control characters.
fn available_keys<V>(table: &HashMap<String, V>) -> String {
    let mut available: Vec<&str> = table.keys().map(String::as_str).collect();
    available.sort_unstable();
    if available.is_empty() {
        return "(none defined)".to_string();
    }
    let escaped: Vec<Cow<'_, str>> = available.into_iter().map(escape_control_chars).collect();
    escaped.join(", ")
}

/// Checks every rule id listed by a policy and every policy id listed by a
/// `policies` middleware instance.
///
/// Ids are matched against the keys of `[rules]` and `[policies]`, because
/// that is how the middleware looks them up. Disabled policies are checked
/// too: the middleware skips them, so a bad id would otherwise stay hidden
/// until someone enables the policy. All errors are returned together,
/// sorted, so one load reports every bad reference.
///
/// The defined ids are listed once per kind, after the error lines. Repeating
/// the list on every line makes the message grow with bad ids times defined
/// ids, which reaches tens of megabytes on a large config.
fn validate_policy_refs(config: &Config) -> Result<(), String> {
    let rule_errors = unknown_rule_errors(config);
    let policy_errors = unknown_policy_errors(config);

    if rule_errors.is_empty() && policy_errors.is_empty() {
        return Ok(());
    }

    let available_rules = (!rule_errors.is_empty())
        .then(|| format!("Available rules: {}", available_keys(&config.rules)));
    let available_policies = (!policy_errors.is_empty())
        .then(|| format!("Available policies: {}", available_keys(&config.policies)));

    let mut lines = rule_errors;
    lines.extend(policy_errors);
    lines.sort();
    lines.extend(available_rules);
    lines.extend(available_policies);
    Err(lines.join("\n"))
}

/// One error per rule id in a `[policies.<key>]` entry that is not a key in
/// `[rules]`.
fn unknown_rule_errors(config: &Config) -> Vec<String> {
    config
        .policies
        .iter()
        .flat_map(|(policy_key, policy)| {
            policy
                .rules
                .iter()
                .map(move |rule_id| (policy_key, rule_id))
        })
        .filter(|(_, rule_id)| !config.rules.contains_key(rule_id.as_str()))
        .map(|(policy_key, rule_id)| {
            format!(
                "Policy '{}' references unknown rule '{}'",
                escape_control_chars(policy_key),
                escape_control_chars(rule_id)
            )
        })
        .collect()
}

/// One error per policy id in the `policies` option of a `policies`
/// middleware instance that is not a key in `[policies]`.
///
/// A missing `policies` option, a non-array value and non-string entries are
/// left to the middleware, which already rejects them.
fn unknown_policy_errors(config: &Config) -> Vec<String> {
    config
        .middleware
        .iter()
        .filter(|(_, instance)| {
            instance
                .middleware_type
                .eq_ignore_ascii_case(POLICIES_MIDDLEWARE_TYPE)
        })
        .flat_map(|(instance_name, instance)| {
            instance
                .options
                .get("policies")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.as_str())
                .map(move |policy_id| (instance_name, policy_id))
        })
        .filter(|(_, policy_id)| !config.policies.contains_key(*policy_id))
        .map(|(instance_name, policy_id)| {
            format!(
                "Middleware '{}' references unknown policy '{}'",
                escape_control_chars(instance_name),
                escape_control_chars(policy_id)
            )
        })
        .collect()
}

fn unknown_authentication_message(
    kind: &str,
    name: &str,
    auth_ref: &str,
    authentications: &HashMap<String, AuthenticationDefinition>,
) -> String {
    let available = available_keys(authentications);
    let name = escape_control_chars(name);
    // The prefix has no control characters, so stripping it before or after
    // escaping gives the same bare key.
    let auth_ref = escape_control_chars(auth_ref);

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
