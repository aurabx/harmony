use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::models::middleware::{AccessDenied, ContentTypeDenied, MethodDenied, RateLimitExceeded};
use crate::utils::Error;
use async_trait::async_trait;
use chrono::Utc;
use ipnetwork::IpNetwork;
use matchit::Router;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Result of evaluating a single rule
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleEvaluation {
    /// Rule matched and indicates allow
    Allow,
    /// Rule matched and indicates deny
    Deny,
    /// Rule did not match
    NoMatch,
}

/// Configuration for a single rule within a policy
#[derive(Debug, Clone)]
pub struct Rule {
    pub id: Option<String>,
    pub name: Option<String>,
    pub rule_type: String,
    pub weight: i64,
    pub enabled: bool,
    pub options: HashMap<String, Value>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            id: None,
            name: None,
            rule_type: String::new(),
            weight: 0,
            enabled: true,
            options: HashMap::new(),
        }
    }
}

/// Configuration for a policy containing multiple rules
#[derive(Debug, Clone)]
pub struct Policy {
    pub id: Option<String>,
    pub name: Option<String>,
    pub enabled: bool,
    pub rules: Vec<Rule>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            id: None,
            name: None,
            enabled: true,
            rules: Vec::new(),
        }
    }
}

/// Top-level configuration for policies middleware
#[derive(Debug, Clone)]
pub struct PoliciesConfig {
    pub policies: Vec<Policy>,
}

/// Compiled IP networks for efficient matching
#[derive(Debug, Clone)]
struct CompiledIpRule {
    networks: Arc<Vec<IpNetwork>>,
}

/// Rate limit state for tracking requests
#[derive(Debug, Clone)]
struct RateLimitState {
    count: u32,
    window_start: i64, // Unix timestamp in seconds
}

/// Compiled path matcher for path rules
#[derive(Debug, Clone)]
struct CompiledPathRule {
    router: Arc<Router<()>>,
    mode: String, // "allow" or "deny"
}

impl CompiledIpRule {
    fn new(ip_addresses: &[String]) -> Result<Self, String> {
        let mut networks = Vec::new();
        for addr_str in ip_addresses {
            let network = IpNetwork::from_str(addr_str).map_err(|e| {
                format!("Invalid IP address or CIDR notation '{}': {}", addr_str, e)
            })?;
            networks.push(network);
        }
        Ok(Self {
            networks: Arc::new(networks),
        })
    }

    /// True when `ip` falls in any of the rule's networks.
    ///
    /// On a dual-stack listener an IPv4 client arrives as an IPv4-mapped IPv6
    /// address (`::ffff:a.b.c.d`), and `IpNetwork::contains` never matches a V6
    /// address against a V4 network. So the canonical (plain IPv4) form is
    /// tested as well as the address as received. Testing both keeps rules
    /// written as IPv6 networks, such as `::ffff:10.0.0.0/104`, matching.
    fn matches(&self, ip: &IpAddr) -> bool {
        let canonical = ip.to_canonical();
        self.networks
            .iter()
            .any(|network| network.contains(canonical) || network.contains(*ip))
    }
}

/// Key that identifies a client in the rate-limit state.
///
/// An IPv4-mapped IPv6 address and its plain IPv4 form are the same client, so
/// both map to the plain IPv4 text. A value that is not an IP address is used
/// as given.
fn rate_limit_client_key(raw_client_ip: &str) -> String {
    match IpAddr::from_str(raw_client_ip) {
        Ok(ip) => ip.to_canonical().to_string(),
        Err(_) => raw_client_ip.to_string(),
    }
}

/// Shared rate-limit state for a single policy instance key.
///
/// Held in a process-wide registry so multiple `PoliciesMiddleware` instances
/// built for the same pipeline share their bucket counts. The `_cleanup`
/// JoinHandle is retained to keep exactly one cleanup task alive per key for
/// the lifetime of the process.
struct SharedRateLimitState {
    buckets: Arc<RwLock<HashMap<String, RateLimitState>>>,
    _cleanup: JoinHandle<()>,
}

static RATE_LIMIT_REGISTRY: Lazy<Mutex<HashMap<String, Arc<SharedRateLimitState>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Look up or create the shared rate-limit state for a given instance key.
///
/// The cleanup task is spawned exactly once per key. Subsequent lookups return
/// the same `Arc<RwLock<..>>` so bucket counts persist across `PoliciesMiddleware`
/// instances rebuilt on each pipeline request.
fn get_or_init_rate_limit_state(
    instance_key: &str,
) -> Arc<RwLock<HashMap<String, RateLimitState>>> {
    let mut registry = RATE_LIMIT_REGISTRY
        .lock()
        .expect("Rate limit registry mutex poisoned");

    if let Some(shared) = registry.get(instance_key) {
        return shared.buckets.clone();
    }

    let buckets: Arc<RwLock<HashMap<String, RateLimitState>>> =
        Arc::new(RwLock::new(HashMap::new()));

    let cleanup = PoliciesMiddleware::spawn_cleanup_task(buckets.clone(), instance_key.to_string());

    let shared = Arc::new(SharedRateLimitState {
        buckets: buckets.clone(),
        _cleanup: cleanup,
    });
    registry.insert(instance_key.to_string(), shared);
    buckets
}

/// Test-only helper: clear the rate-limit registry.
///
/// Exposed for the crate's own tests and integration tests that need to reset
/// bucket state between runs. Not part of the public API.
#[doc(hidden)]
pub fn clear_rate_limit_registry_for_tests() {
    let mut registry = RATE_LIMIT_REGISTRY
        .lock()
        .expect("Rate limit registry mutex poisoned");
    registry.clear();
}

/// Policies middleware implementation
pub struct PoliciesMiddleware {
    policies: Vec<Policy>,
    // Pre-compiled IP rules for performance
    compiled_ip_rules: HashMap<usize, CompiledIpRule>, // policy_idx -> rule_idx key
    // Pre-compiled path rules for performance
    compiled_path_rules: HashMap<usize, CompiledPathRule>,
    // Rate limit state tracking (client_ip + rule_key -> state).
    // Shared across all instances constructed with the same instance_key via
    // the process-wide RATE_LIMIT_REGISTRY.
    rate_limit_state: Arc<RwLock<HashMap<String, RateLimitState>>>,
}

impl PoliciesMiddleware {
    /// Construct a `PoliciesMiddleware` whose rate-limit state is not shared with
    /// any other instance.
    ///
    /// Used by tests and any caller that does not have a stable pipeline / instance
    /// identifier. Each call receives a fresh bucket map and its own cleanup task.
    pub fn new(config: PoliciesConfig) -> Result<Self, String> {
        let unique_key = format!("anon-{}", uuid::Uuid::new_v4());
        Self::with_instance_key(config, &unique_key)
    }

    /// Construct a `PoliciesMiddleware` that shares rate-limit state with any other
    /// instance built with the same `instance_key`.
    ///
    /// The middleware factory calls this with the middleware instance name from
    /// configuration so rebuilds across requests keep the running counts.
    pub fn with_instance_key(config: PoliciesConfig, instance_key: &str) -> Result<Self, String> {
        if config.policies.is_empty() {
            return Err("Policies middleware requires at least one policy".to_string());
        }

        // Filter to enabled policies only
        let enabled_policies: Vec<Policy> =
            config.policies.into_iter().filter(|p| p.enabled).collect();

        if enabled_policies.is_empty() {
            return Err("Policies middleware requires at least one enabled policy".to_string());
        }

        let mut compiled_ip_rules = HashMap::new();
        let mut compiled_path_rules = HashMap::new();

        // Pre-compile IP and path rules for all policies
        for (policy_idx, policy) in enabled_policies.iter().enumerate() {
            for (rule_idx, rule) in policy.rules.iter().enumerate() {
                if !rule.enabled {
                    continue;
                }

                match rule.rule_type.as_str() {
                    "ip_allow" | "ip_deny" => {
                        let ip_addresses = rule
                            .options
                            .get("ip_addresses")
                            .and_then(|v| v.as_array())
                            .ok_or_else(|| {
                                format!(
                                    "Rule '{}' (type {}) missing required 'ip_addresses' array",
                                    rule.name.as_deref().unwrap_or("unnamed"),
                                    rule.rule_type
                                )
                            })?;

                        let ip_strings: Vec<String> = ip_addresses
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();

                        let compiled = CompiledIpRule::new(&ip_strings)?;
                        let key = policy_idx * 10000 + rule_idx; // Simple composite key
                        compiled_ip_rules.insert(key, compiled);
                    }
                    "path" => {
                        let paths = rule
                            .options
                            .get("paths")
                            .and_then(|v| v.as_array())
                            .ok_or_else(|| {
                                format!(
                                    "Rule '{}' (type path) missing required 'paths' array",
                                    rule.name.as_deref().unwrap_or("unnamed")
                                )
                            })?;

                        let mode = rule
                            .options
                            .get("mode")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                format!(
                                    "Rule '{}' (type path) missing required 'mode' field",
                                    rule.name.as_deref().unwrap_or("unnamed")
                                )
                            })?;

                        if mode != "allow" && mode != "deny" {
                            return Err(format!(
                                "Rule '{}' (type path) mode must be 'allow' or 'deny', got '{}'",
                                rule.name.as_deref().unwrap_or("unnamed"),
                                mode
                            ));
                        }

                        let mut router = Router::new();
                        for (path_idx, path_val) in paths.iter().enumerate() {
                            let path_str = path_val.as_str().ok_or_else(|| {
                                format!(
                                    "Rule '{}' path at index {} must be a string",
                                    rule.name.as_deref().unwrap_or("unnamed"),
                                    path_idx
                                )
                            })?;

                            if !path_str.starts_with('/') {
                                return Err(format!(
                                    "Rule '{}' path '{}' must start with '/'",
                                    rule.name.as_deref().unwrap_or("unnamed"),
                                    path_str
                                ));
                            }

                            router.insert(path_str, ()).map_err(|e| {
                                format!(
                                    "Rule '{}' failed to compile path '{}': {}",
                                    rule.name.as_deref().unwrap_or("unnamed"),
                                    path_str,
                                    e
                                )
                            })?;
                        }

                        let key = policy_idx * 10000 + rule_idx;
                        compiled_path_rules.insert(
                            key,
                            CompiledPathRule {
                                router: Arc::new(router),
                                mode: mode.to_string(),
                            },
                        );
                    }
                    _ => {} // Other rule types don't need pre-compilation
                }
            }
        }

        tracing::info!(
            "PoliciesMiddleware initialized with {} enabled policies (instance_key={})",
            enabled_policies.len(),
            instance_key
        );

        // Share bucket state across all PoliciesMiddleware instances built with
        // the same instance_key. The registry starts the cleanup task exactly
        // once per key.
        let rate_limit_state = get_or_init_rate_limit_state(instance_key);

        Ok(Self {
            policies: enabled_policies,
            compiled_ip_rules,
            compiled_path_rules,
            rate_limit_state,
        })
    }

    /// Spawn a background task to periodically clean up expired rate limit entries.
    ///
    /// Only called once per instance key by `get_or_init_rate_limit_state`; the
    /// returned `JoinHandle` is held by the registry to keep the task alive.
    fn spawn_cleanup_task(
        state: Arc<RwLock<HashMap<String, RateLimitState>>>,
        instance_key: String,
    ) -> JoinHandle<()> {
        let task_key = instance_key.clone();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

            loop {
                interval.tick().await;

                let mut state_map = state.write().await;
                let current_time = Utc::now().timestamp();
                let initial_size = state_map.len();

                // Remove entries older than 5 minutes (300 seconds).
                // Conservative window - outlasts most rate limit windows so a
                // slow client's count is not wiped mid-window.
                state_map.retain(|_key, state| current_time - state.window_start < 300);

                let removed = initial_size - state_map.len();
                if removed > 0 {
                    tracing::debug!(
                        "Rate limit cleanup: removed {} expired entries, {} remaining (instance_key={})",
                        removed,
                        state_map.len(),
                        task_key
                    );
                }
            }
        });

        tracing::info!(
            "Rate limit cleanup task started for instance_key={} (runs every 60 seconds)",
            instance_key
        );

        handle
    }

    /// Enforce the IP allowlist gate of the middleware (AURA-2438).
    ///
    /// Every enabled `ip_allow` rule in every policy of this middleware forms
    /// one allowlist. The rules are a union, within a policy and across
    /// policies, so "office OR VPN" can be two rules or two policies. If any
    /// such rule exists, the client IP must match at least one of them. If none
    /// exists, there is no gate. Without this check any other allow rule, such
    /// as a method rule in allow mode, admitted clients from outside the
    /// allowlist, because the combined result only needs one allow and no deny.
    ///
    /// Fails closed: when an allowlist exists, a request whose envelope has no
    /// parseable client IP is denied. HTTP/3 does not record the peer address
    /// yet (AURA-2417), so HTTP/3 requests always fail this gate.
    ///
    /// Returns `AccessDenied` (HTTP 403) when the gate fails. Reads the
    /// envelope only; it has no side effects besides logging.
    fn check_ip_allowlists(
        &self,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> Result<(), Error> {
        let allowlist: Vec<(usize, usize, &Rule)> = self
            .policies
            .iter()
            .enumerate()
            .flat_map(|(policy_idx, policy)| {
                policy
                    .rules
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.enabled && r.rule_type == "ip_allow")
                    .map(move |(rule_idx, rule)| (policy_idx, rule_idx, rule))
            })
            .collect();
        if allowlist.is_empty() {
            return Ok(());
        }

        let raw_client_ip = envelope
            .request_details
            .metadata
            .get("remote_addr")
            .or_else(|| envelope.request_details.metadata.get("client_ip"));
        let client_ip = raw_client_ip.and_then(|raw| IpAddr::from_str(raw).ok());

        let rule_label = |rule: &Rule| -> String {
            rule.id
                .as_deref()
                .or(rule.name.as_deref())
                .unwrap_or("unnamed")
                .to_string()
        };
        let rule_ids = allowlist
            .iter()
            .map(|(_, _, r)| rule_label(r))
            .collect::<Vec<_>>()
            .join(", ");

        let Some(ip) = client_ip else {
            tracing::warn!(
                "Request DENIED - an IP allowlist is configured (rules: {}) but the client IP is unknown (remote_addr={:?}); failing closed",
                rule_ids,
                raw_client_ip
            );
            return Err(Box::new(AccessDenied(
                "Client IP is unknown and an IP allowlist match is required".to_string(),
            )));
        };

        let matched_rule = allowlist.iter().find(|(policy_idx, rule_idx, _)| {
            let key = policy_idx * 10000 + rule_idx;
            match self.compiled_ip_rules.get(&key) {
                Some(compiled) => compiled.matches(&ip),
                None => {
                    tracing::error!(
                        "Compiled IP rule not found for key {} - this is a bug; treating as no match",
                        key
                    );
                    false
                }
            }
        });

        match matched_rule {
            Some((policy_idx, _, rule)) => {
                let policy = &self.policies[*policy_idx];
                tracing::debug!(
                    "IP allowlist gate passed: policy='{}', rule='{}', client_ip={}",
                    policy
                        .id
                        .as_deref()
                        .or(policy.name.as_deref())
                        .unwrap_or("unnamed"),
                    rule_label(rule),
                    ip
                );
                Ok(())
            }
            None => {
                tracing::info!(
                    "Request DENIED - client_ip={} matches none of the IP allowlist rules (rules: {})",
                    ip,
                    rule_ids
                );
                Err(Box::new(AccessDenied(
                    "Client IP is not in the IP allowlist".to_string(),
                )))
            }
        }
    }

    /// Evaluate a single rule against the request envelope
    async fn evaluate_rule(
        &self,
        rule: &Rule,
        policy_idx: usize,
        rule_idx: usize,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        tracing::debug!(
            "Evaluating rule: type={}, name={:?}, weight={}",
            rule.rule_type,
            rule.name,
            rule.weight
        );

        match rule.rule_type.as_str() {
            "ip_allow" => self.evaluate_ip_rule(rule, policy_idx, rule_idx, envelope, true),
            "ip_deny" => self.evaluate_ip_rule(rule, policy_idx, rule_idx, envelope, false),
            "allow_all" => RuleEvaluation::Allow,
            "deny_all" => RuleEvaluation::Deny,
            "rate_limit" => {
                self.evaluate_rate_limit_rule(rule, policy_idx, rule_idx, envelope)
                    .await
            }
            "path" => self.evaluate_path_rule(rule, policy_idx, rule_idx, envelope),
            "geo" => self.evaluate_geo_rule(rule, envelope),
            "header" => self.evaluate_header_rule(rule, envelope),
            "time_based" => self.evaluate_time_based_rule(rule),
            "method" => self.evaluate_method_rule(rule, envelope),
            "user_agent" => self.evaluate_user_agent_rule(rule, envelope),
            "content_type" => self.evaluate_content_type_rule(rule, envelope),
            "query_parameter" => self.evaluate_query_parameter_rule(rule, envelope),
            _ => {
                tracing::warn!(
                    "Unknown rule type '{}' - treating as no match",
                    rule.rule_type
                );
                RuleEvaluation::NoMatch
            }
        }
    }

    /// Evaluate IP-based rules (allow or deny)
    fn evaluate_ip_rule(
        &self,
        _rule: &Rule,
        policy_idx: usize,
        rule_idx: usize,
        envelope: &RequestEnvelope<serde_json::Value>,
        is_allow: bool,
    ) -> RuleEvaluation {
        // Extract client IP from metadata
        let client_ip_str = envelope
            .request_details
            .metadata
            .get("remote_addr")
            .or_else(|| envelope.request_details.metadata.get("client_ip"));

        let client_ip_str = match client_ip_str {
            Some(ip) => ip,
            None => {
                tracing::debug!("No client IP found in metadata, rule does not match");
                return RuleEvaluation::NoMatch;
            }
        };

        // Parse client IP
        let client_ip = match IpAddr::from_str(client_ip_str) {
            Ok(ip) => ip,
            Err(e) => {
                tracing::warn!(
                    "Failed to parse client IP '{}': {} - rule does not match",
                    client_ip_str,
                    e
                );
                return RuleEvaluation::NoMatch;
            }
        };

        // Get compiled rule
        let key = policy_idx * 10000 + rule_idx;
        let compiled_rule = match self.compiled_ip_rules.get(&key) {
            Some(r) => r,
            None => {
                tracing::error!("Compiled IP rule not found for key {} - this is a bug", key);
                return RuleEvaluation::NoMatch;
            }
        };

        // Check if client IP matches any network
        let matches = compiled_rule.matches(&client_ip);

        tracing::debug!(
            "IP rule evaluation: client_ip={}, matches={}, is_allow={}",
            client_ip,
            matches,
            is_allow
        );

        if matches {
            if is_allow {
                RuleEvaluation::Allow
            } else {
                RuleEvaluation::Deny
            }
        } else {
            RuleEvaluation::NoMatch
        }
    }

    /// Evaluate rate limiting rule
    async fn evaluate_rate_limit_rule(
        &self,
        rule: &Rule,
        policy_idx: usize,
        rule_idx: usize,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        // Extract configuration
        let max_requests = rule
            .options
            .get("max_requests")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as u32;

        let window_seconds = rule
            .options
            .get("window_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(60) as u32;

        // Get client identifier (IP)
        let client_ip = envelope
            .request_details
            .metadata
            .get("remote_addr")
            .or_else(|| envelope.request_details.metadata.get("client_ip"))
            .map(|raw| rate_limit_client_key(raw))
            .unwrap_or_else(|| "unknown".to_string());

        // Create unique key for this rate limit rule
        let rate_key = format!("{}:{}:{}", client_ip, policy_idx, rule_idx);

        let current_time = Utc::now().timestamp();
        let mut state_map = self.rate_limit_state.write().await;

        let state = state_map.entry(rate_key.clone()).or_insert(RateLimitState {
            count: 0,
            window_start: current_time,
        });

        // Check if window has expired
        if current_time - state.window_start >= window_seconds as i64 {
            // Reset window
            state.count = 1;
            state.window_start = current_time;
            tracing::debug!(
                "Rate limit window reset for {}: 1/{} requests",
                client_ip,
                max_requests
            );
            RuleEvaluation::NoMatch // Within limit
        } else if state.count >= max_requests {
            // Rate limit exceeded
            tracing::warn!(
                "Rate limit exceeded for {}: {}/{} requests in window",
                client_ip,
                state.count,
                max_requests
            );
            RuleEvaluation::Deny // Acts as deny when limit exceeded
        } else {
            // Within limit, increment counter
            state.count += 1;
            tracing::debug!(
                "Rate limit check for {}: {}/{} requests",
                client_ip,
                state.count,
                max_requests
            );
            RuleEvaluation::NoMatch
        }
    }

    /// Evaluate path matching rule
    fn evaluate_path_rule(
        &self,
        _rule: &Rule,
        policy_idx: usize,
        rule_idx: usize,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        // Get request path from metadata
        let request_path = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_else(|| "/".to_string());

        // Normalize path
        let normalized_path = if request_path.is_empty() {
            "/".to_string()
        } else if !request_path.starts_with('/') {
            format!("/{}", request_path)
        } else {
            request_path.clone()
        };

        // Get compiled rule
        let key = policy_idx * 10000 + rule_idx;
        let compiled_rule = match self.compiled_path_rules.get(&key) {
            Some(r) => r,
            None => {
                tracing::error!(
                    "Compiled path rule not found for key {} - this is a bug",
                    key
                );
                return RuleEvaluation::NoMatch;
            }
        };

        // Check if path matches
        let matches = compiled_rule.router.at(&normalized_path).is_ok();

        tracing::debug!(
            "Path rule evaluation: path={}, matches={}, mode={}",
            normalized_path,
            matches,
            compiled_rule.mode
        );

        if matches {
            match compiled_rule.mode.as_str() {
                "allow" => RuleEvaluation::Allow,
                "deny" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        } else {
            RuleEvaluation::NoMatch
        }
    }

    /// Evaluate geographic rule
    fn evaluate_geo_rule(
        &self,
        rule: &Rule,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        // Extract configuration
        let country_codes = rule
            .options
            .get("country_codes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_uppercase())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let mode = rule
            .options
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("deny");

        // Get country code from metadata
        let client_country = envelope
            .request_details
            .metadata
            .get("geo_country")
            .or_else(|| envelope.request_details.metadata.get("country_code"))
            .map(|s| s.to_uppercase());

        let client_country = match client_country {
            Some(cc) => cc,
            None => {
                tracing::debug!("No geo_country found in metadata");
                return RuleEvaluation::NoMatch;
            }
        };

        let matches = country_codes.contains(&client_country);

        tracing::debug!(
            "Geo rule evaluation: country={}, matches={}, mode={}",
            client_country,
            matches,
            mode
        );

        if matches {
            match mode {
                "allow" => RuleEvaluation::Allow,
                "deny" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        } else {
            RuleEvaluation::NoMatch
        }
    }

    /// Evaluate header matching rule
    fn evaluate_header_rule(
        &self,
        rule: &Rule,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        let mode = rule
            .options
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("deny");

        let headers = match rule.options.get("headers").and_then(|v| v.as_array()) {
            Some(h) => h,
            None => return RuleEvaluation::NoMatch,
        };

        if headers.is_empty() {
            return RuleEvaluation::NoMatch;
        }

        // Check if all configured headers match
        let mut all_match = true;

        for header_config in headers {
            let header_obj = match header_config.as_object() {
                Some(obj) => obj,
                None => continue,
            };

            let header_name = match header_obj.get("name").and_then(|v| v.as_str()) {
                Some(n) => n,
                None => continue,
            };

            let match_type = header_obj
                .get("match_type")
                .and_then(|v| v.as_str())
                .unwrap_or("exact");

            let expected_value = match header_obj.get("value").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => continue,
            };

            // Get actual header value from metadata (headers are typically stored as "header_<name>")
            let metadata_key = format!("header_{}", header_name.to_lowercase());
            let actual_value = envelope
                .request_details
                .metadata
                .get(&metadata_key)
                .or_else(|| envelope.request_details.metadata.get(header_name));

            let actual_value = match actual_value {
                Some(v) => v,
                None => {
                    all_match = false;
                    break;
                }
            };

            // Apply matching logic
            let header_matches = match match_type {
                "exact" => actual_value.eq_ignore_ascii_case(expected_value),
                "contains" => actual_value
                    .to_lowercase()
                    .contains(&expected_value.to_lowercase()),
                "regex" => match Regex::new(expected_value) {
                    Ok(re) => re.is_match(actual_value),
                    Err(e) => {
                        tracing::warn!("Invalid regex pattern '{}': {}", expected_value, e);
                        false
                    }
                },
                _ => false,
            };

            if !header_matches {
                all_match = false;
                break;
            }
        }

        tracing::debug!(
            "Header rule evaluation: all_match={}, mode={}",
            all_match,
            mode
        );

        if all_match {
            match mode {
                "allow" => RuleEvaluation::Allow,
                "deny" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        } else {
            RuleEvaluation::NoMatch
        }
    }

    /// Evaluate time-based rule
    fn evaluate_time_based_rule(&self, rule: &Rule) -> RuleEvaluation {
        use chrono::{Datelike, NaiveTime};
        use chrono_tz::Tz;

        // Get configuration
        let allow_during_window = rule
            .options
            .get("allow_during_window")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let timezone_str = rule
            .options
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("UTC");

        // Parse timezone
        let timezone: Tz = match timezone_str.parse() {
            Ok(tz) => tz,
            Err(e) => {
                tracing::warn!("Invalid timezone '{}': {} - using UTC", timezone_str, e);
                "UTC".parse().unwrap()
            }
        };

        // Get current time in specified timezone
        let now = Utc::now().with_timezone(&timezone);

        // Check time window (HH:MM format)
        if let (Some(start_time_str), Some(end_time_str)) = (
            rule.options.get("start_time").and_then(|v| v.as_str()),
            rule.options.get("end_time").and_then(|v| v.as_str()),
        ) {
            let start_time = match NaiveTime::parse_from_str(start_time_str, "%H:%M") {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        "Invalid start_time '{}': {} - skipping time check",
                        start_time_str,
                        e
                    );
                    return RuleEvaluation::NoMatch;
                }
            };

            let end_time = match NaiveTime::parse_from_str(end_time_str, "%H:%M") {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        "Invalid end_time '{}': {} - skipping time check",
                        end_time_str,
                        e
                    );
                    return RuleEvaluation::NoMatch;
                }
            };

            let current_time = now.time();

            // Check if current time is within window
            let in_time_window = if start_time <= end_time {
                // Normal case: 09:00 to 17:00
                current_time >= start_time && current_time <= end_time
            } else {
                // Wraps midnight: 22:00 to 06:00
                current_time >= start_time || current_time <= end_time
            };

            if !in_time_window {
                tracing::debug!(
                    "Time-based rule: current time {} outside window {}-{}",
                    current_time.format("%H:%M"),
                    start_time_str,
                    end_time_str
                );
                return if allow_during_window {
                    // Outside window and rule allows during window -> deny
                    RuleEvaluation::Deny
                } else {
                    // Outside window and rule denies during window -> allow
                    RuleEvaluation::Allow
                };
            }
        }

        // Check day of week
        if let Some(days_array) = rule.options.get("days_of_week").and_then(|v| v.as_array()) {
            let allowed_days: Vec<String> = days_array
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                .collect();

            if !allowed_days.is_empty() {
                let current_day = now.weekday().to_string().to_lowercase();

                if !allowed_days.contains(&current_day) {
                    tracing::debug!(
                        "Time-based rule: current day {} not in allowed days {:?}",
                        current_day,
                        allowed_days
                    );
                    return if allow_during_window {
                        RuleEvaluation::Deny
                    } else {
                        RuleEvaluation::Allow
                    };
                }
            }
        }

        // Check date range (YYYY-MM-DD format)
        if let (Some(start_date_str), Some(end_date_str)) = (
            rule.options.get("start_date").and_then(|v| v.as_str()),
            rule.options.get("end_date").and_then(|v| v.as_str()),
        ) {
            use chrono::NaiveDate;

            let start_date = match NaiveDate::parse_from_str(start_date_str, "%Y-%m-%d") {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        "Invalid start_date '{}': {} - skipping date check",
                        start_date_str,
                        e
                    );
                    return RuleEvaluation::NoMatch;
                }
            };

            let end_date = match NaiveDate::parse_from_str(end_date_str, "%Y-%m-%d") {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        "Invalid end_date '{}': {} - skipping date check",
                        end_date_str,
                        e
                    );
                    return RuleEvaluation::NoMatch;
                }
            };

            let current_date = now.date_naive();

            if current_date < start_date || current_date > end_date {
                tracing::debug!(
                    "Time-based rule: current date {} outside range {}-{}",
                    current_date,
                    start_date_str,
                    end_date_str
                );
                return if allow_during_window {
                    RuleEvaluation::Deny
                } else {
                    RuleEvaluation::Allow
                };
            }
        }

        // All conditions passed - within time window
        tracing::debug!("Time-based rule: within allowed time window");
        if allow_during_window {
            RuleEvaluation::Allow
        } else {
            RuleEvaluation::Deny
        }
    }
    /// Evaluate HTTP method rule
    fn evaluate_method_rule(
        &self,
        rule: &Rule,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        let mode = rule
            .options
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("deny");

        let methods = match rule.options.get("methods").and_then(|v| v.as_array()) {
            Some(m) => m,
            None => return RuleEvaluation::NoMatch,
        };

        if methods.is_empty() {
            return RuleEvaluation::NoMatch;
        }

        // Extract HTTP method from request
        let request_method = envelope.request_details.method.to_uppercase();

        // Check if method matches any in the list (case-insensitive)
        let matches = methods.iter().any(|m| {
            m.as_str()
                .map(|s| s.to_uppercase() == request_method)
                .unwrap_or(false)
        });

        tracing::debug!(
            "Method rule evaluation: method={}, matches={}, mode={}",
            request_method,
            matches,
            mode
        );

        if matches {
            match mode {
                "allow" => RuleEvaluation::Allow,
                "deny" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        } else {
            // If mode is "allow" and method doesn't match, deny the request
            // If mode is "deny" and method doesn't match, don't evaluate (NoMatch)
            match mode {
                "allow" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        }
    }

    /// Evaluate User-Agent rule with regex pattern matching
    fn evaluate_user_agent_rule(
        &self,
        rule: &Rule,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        let mode = rule
            .options
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("deny");

        let patterns = match rule.options.get("patterns").and_then(|v| v.as_array()) {
            Some(p) => p,
            None => return RuleEvaluation::NoMatch,
        };

        if patterns.is_empty() {
            return RuleEvaluation::NoMatch;
        }

        // Extract User-Agent header
        let user_agent = envelope.request_details.headers.get("user-agent").cloned();

        let user_agent = match user_agent {
            Some(ua) => ua,
            None => {
                tracing::debug!("No user-agent header found in request");
                return RuleEvaluation::NoMatch;
            }
        };

        // Check if any pattern matches
        let mut any_match = false;
        for pattern_config in patterns {
            let pattern_obj = match pattern_config.as_object() {
                Some(obj) => obj,
                None => continue,
            };

            let pattern_str = match pattern_obj.get("pattern").and_then(|v| v.as_str()) {
                Some(p) => p,
                None => continue,
            };

            // Parse pattern (format: /pattern/flags or just pattern)
            let (pattern, flags) = if pattern_str.starts_with('/')
                && pattern_str.rfind('/').map(|i| i > 0).unwrap_or(false)
            {
                let last_slash = pattern_str.rfind('/').unwrap();
                (&pattern_str[1..last_slash], &pattern_str[last_slash + 1..])
            } else {
                (pattern_str, "")
            };

            // Build regex with flags
            let regex_str = if flags.contains('i') {
                format!("(?i){}", pattern)
            } else {
                pattern.to_string()
            };

            match Regex::new(&regex_str) {
                Ok(re) => {
                    if re.is_match(&user_agent) {
                        tracing::debug!(
                            "User-agent pattern matched: pattern='{}', user-agent='{}'",
                            pattern_obj
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or(pattern),
                            user_agent
                        );
                        any_match = true;
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Invalid regex pattern '{}': {} - skipping pattern",
                        pattern,
                        e
                    );
                    continue;
                }
            }
        }

        tracing::debug!(
            "User-agent rule evaluation: any_match={}, mode={}",
            any_match,
            mode
        );

        if any_match {
            match mode {
                "allow" => RuleEvaluation::Allow,
                "deny" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        } else {
            RuleEvaluation::NoMatch
        }
    }

    /// Evaluate Content-Type rule with wildcard support
    fn evaluate_content_type_rule(
        &self,
        rule: &Rule,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        let mode = rule
            .options
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("deny");

        let content_types = match rule.options.get("content_types").and_then(|v| v.as_array()) {
            Some(ct) => ct,
            None => return RuleEvaluation::NoMatch,
        };

        if content_types.is_empty() {
            return RuleEvaluation::NoMatch;
        }

        // Extract Content-Type header
        let content_type = envelope
            .request_details
            .headers
            .get("content-type")
            .cloned();

        let content_type = match content_type {
            Some(ct) => ct,
            None => {
                tracing::debug!("No content-type header found in request");
                return RuleEvaluation::NoMatch;
            }
        };

        // Strip charset and parameters from Content-Type
        let base_content_type = content_type
            .split(';')
            .next()
            .unwrap_or(&content_type)
            .trim();

        // Check if any configured content type matches
        let mut exact_match = false;
        let mut wildcard_match = false;

        for ct_config in content_types {
            let ct_str = match ct_config.as_str() {
                Some(s) => s,
                None => continue,
            };

            // Exact match
            if ct_str == base_content_type {
                exact_match = true;
                break;
            }

            // Wildcard match (e.g., "application/*")
            if ct_str.ends_with("/*") {
                let prefix = &ct_str[..ct_str.len() - 2];
                if base_content_type.starts_with(prefix)
                    && base_content_type.chars().nth(prefix.len()) == Some('/')
                {
                    wildcard_match = true;
                }
            }
            // Catch-all wildcard
            else if ct_str == "*/*" {
                wildcard_match = true;
            }
        }

        let matches = exact_match || wildcard_match;

        tracing::debug!(
            "Content-type rule evaluation: content_type={}, matches={}, mode={}",
            base_content_type,
            matches,
            mode
        );

        if matches {
            match mode {
                "allow" => RuleEvaluation::Allow,
                "deny" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        } else {
            // If mode is "allow" and content-type doesn't match, deny the request
            // If mode is "deny" and content-type doesn't match, don't evaluate (NoMatch)
            match mode {
                "allow" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        }
    }

    /// Evaluate Query Parameter rule with multiple match types
    fn evaluate_query_parameter_rule(
        &self,
        rule: &Rule,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> RuleEvaluation {
        let mode = rule
            .options
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("deny");

        let parameters = match rule.options.get("parameters").and_then(|v| v.as_array()) {
            Some(p) => p,
            None => return RuleEvaluation::NoMatch,
        };

        if parameters.is_empty() {
            return RuleEvaluation::NoMatch;
        }

        // Extract query parameters from metadata
        // Query parameters may be stored as "query_params" or individual "param_<name>" entries
        let query_params = envelope
            .request_details
            .metadata
            .get("query_params")
            .cloned()
            .unwrap_or_else(|| String::new());

        // Parse query parameters manually or from metadata
        let mut param_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (key, value) in &envelope.request_details.metadata {
            if key.starts_with("param_") {
                let param_name = key[6..].to_string();
                param_map.insert(param_name, value.clone());
            }
        }

        // Also parse from query_params string if present (format: key1=value1&key2=value2)
        if !query_params.is_empty() {
            for pair in query_params.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    param_map.insert(key.to_string(), value.to_string());
                }
            }
        }

        // Evaluate all parameters with AND logic
        let mut all_match = true;

        for param_config in parameters {
            let param_obj = match param_config.as_object() {
                Some(obj) => obj,
                None => continue,
            };

            let param_name = match param_obj.get("name").and_then(|v| v.as_str()) {
                Some(n) => n,
                None => continue,
            };

            let match_type = param_obj
                .get("match_type")
                .and_then(|v| v.as_str())
                .unwrap_or("exact");

            let expected_value = param_obj
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let actual_value = param_map.get(param_name);

            // Evaluate match based on match_type
            let param_matches = match match_type {
                "exists" => actual_value.is_some(),
                "exact" => actual_value.map(|v| v == expected_value).unwrap_or(false),
                "contains" => actual_value
                    .map(|v| v.contains(expected_value))
                    .unwrap_or(false),
                "regex" => match actual_value {
                    Some(v) => match Regex::new(expected_value) {
                        Ok(re) => re.is_match(v),
                        Err(e) => {
                            tracing::warn!(
                                "Invalid regex pattern '{}' for parameter '{}': {}",
                                expected_value,
                                param_name,
                                e
                            );
                            false
                        }
                    },
                    None => false,
                },
                _ => false,
            };

            if !param_matches {
                tracing::debug!(
                    "Query parameter rule: parameter '{}' match_type='{}' did not match",
                    param_name,
                    match_type
                );
                all_match = false;
                break; // Short-circuit: AND logic, first failure means overall failure
            }
        }

        tracing::debug!(
            "Query parameter rule evaluation: all_match={}, mode={}",
            all_match,
            mode
        );

        if all_match {
            match mode {
                "allow" => RuleEvaluation::Allow,
                "deny" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        } else {
            // If mode is "allow" and parameters don't match, deny the request
            // If mode is "deny" and parameters don't match, don't evaluate (NoMatch)
            match mode {
                "allow" => RuleEvaluation::Deny,
                _ => RuleEvaluation::NoMatch,
            }
        }
    }
}

#[async_trait]
impl Middleware for PoliciesMiddleware {
    async fn left(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // The allowlist gate runs before the other rules, so a client outside
        // the allowlist never consumes rate-limit budget.
        self.check_ip_allowlists(&envelope)?;

        let mut has_allow = false;
        let mut deny_rule: Option<&Rule> = None;

        // Evaluate all enabled policies and rules
        for (policy_idx, policy) in self.policies.iter().enumerate() {
            let policy_name = policy
                .name
                .as_deref()
                .or(policy.id.as_deref())
                .unwrap_or("unnamed");

            tracing::debug!("Evaluating policy: {}", policy_name);

            // Get enabled rules and sort by weight (descending - higher weight first)
            let mut enabled_rules: Vec<(usize, &Rule)> = policy
                .rules
                .iter()
                .enumerate()
                .filter(|(_, r)| r.enabled)
                .collect();

            enabled_rules.sort_by(|(_, a), (_, b)| b.weight.cmp(&a.weight));

            // Evaluate all rules
            for (rule_idx, rule) in enabled_rules {
                let evaluation = self
                    .evaluate_rule(rule, policy_idx, rule_idx, &envelope)
                    .await;

                match evaluation {
                    RuleEvaluation::Allow => {
                        has_allow = true;
                        tracing::debug!(
                            "Rule {:?} matched - ALLOW",
                            rule.name.as_deref().unwrap_or("unnamed")
                        );
                    }
                    RuleEvaluation::Deny => {
                        deny_rule = Some(rule);
                        tracing::warn!(
                            "Rule {:?} (type: {}) matched - DENY",
                            rule.name.as_deref().unwrap_or("unnamed"),
                            rule.rule_type
                        );
                        // Don't break - continue evaluating to capture all allows/denies
                    }
                    RuleEvaluation::NoMatch => {
                        tracing::debug!(
                            "Rule {:?} did not match",
                            rule.name.as_deref().unwrap_or("unnamed")
                        );
                    }
                }
            }
        }

        // Apply evaluation logic:
        // Request is ACCEPTED only if: the IP allowlist gate passed (above),
        // has_allow = true AND deny_rule = None
        if let Some(rule) = deny_rule {
            let rule_name = rule.name.as_deref().unwrap_or("unnamed");
            tracing::warn!(
                "Request DENIED by rule '{}' (type: {})",
                rule_name,
                rule.rule_type
            );

            // Return appropriate error based on rule type
            return Err(match rule.rule_type.as_str() {
                "rate_limit" => Box::new(RateLimitExceeded(rule_name.to_string())) as Error,
                "method" => Box::new(MethodDenied(format!(
                    "Method not allowed by rule '{}'",
                    rule_name
                ))) as Error,
                "content_type" => Box::new(ContentTypeDenied(format!(
                    "Content type not allowed by rule '{}'",
                    rule_name
                ))) as Error,
                _ => Box::new(AccessDenied(format!(
                    "Access denied by rule '{}'",
                    rule_name
                ))) as Error,
            });
        } else if !has_allow {
            tracing::warn!("Request DENIED - no allow rule matched (implicit deny)");
            return Err(Box::new(AccessDenied(
                "No policy rule allows this request".to_string(),
            )) as Error);
        }

        tracing::debug!("Request ALLOWED - has allow rule(s) and no deny rules");
        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        // Policies only apply on the left (incoming requests)
        Ok(envelope)
    }
}

/// Parse configuration from HashMap for middleware registry
pub fn parse_config(
    options: &HashMap<String, Value>,
    all_policies: &std::collections::HashMap<String, crate::config::config::PolicyDefinition>,
    all_rules: &std::collections::HashMap<String, crate::config::config::RuleDefinition>,
) -> Result<PoliciesConfig, String> {
    let policies_array = options
        .get("policies")
        .and_then(|v| v.as_array())
        .ok_or("Missing required 'policies' array in policies middleware config")?;

    if policies_array.is_empty() {
        return Err("Policies array cannot be empty".to_string());
    }

    // v1.7.0: policies array contains string IDs only
    let mut policies = Vec::new();

    for policy_id_value in policies_array {
        let policy_id = policy_id_value
            .as_str()
            .ok_or("All entries in 'policies' must be string IDs")?;

        let pdef = all_policies
            .get(policy_id)
            .ok_or_else(|| format!("Unknown policy id '{}'", policy_id))?;

        if !pdef.enabled {
            continue;
        }

        let mut rules = Vec::new();
        for rule_id in &pdef.rules {
            let rdef = all_rules.get(rule_id).ok_or_else(|| {
                format!(
                    "Unknown rule id '{}' referenced by policy '{}'",
                    rule_id, pdef.id
                )
            })?;

            if !rdef.enabled {
                continue;
            }

            rules.push(Rule {
                id: Some(rdef.id.clone()),
                name: rdef.name.clone(),
                rule_type: rdef.rule_type.clone(),
                weight: rdef.weight,
                enabled: rdef.enabled,
                options: rdef.options.clone(),
            });
        }

        policies.push(Policy {
            id: Some(pdef.id.clone()),
            name: pdef.name.clone(),
            enabled: pdef.enabled,
            rules,
        });
    }

    Ok(PoliciesConfig { policies })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::RequestEnvelopeBuilder;

    fn create_test_envelope(client_ip: &str) -> RequestEnvelope<serde_json::Value> {
        RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/test")
            .metadata_entry("remote_addr", client_ip)
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_allow_rule_no_deny_accepts() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec![Rule {
                    id: Some("allow_rule".to_string()),
                    name: Some("Allow Internal".to_string()),
                    rule_type: "ip_allow".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "ip_addresses".to_string(),
                            serde_json::json!(["192.168.1.0/24"]),
                        );
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("192.168.1.100");
        let result = middleware.left(envelope).await.unwrap();

        // Should not set skip_backends (request allowed)
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_allow_plus_deny_denies() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec![
                    Rule {
                        id: Some("allow_rule".to_string()),
                        name: Some("Allow Internal".to_string()),
                        rule_type: "ip_allow".to_string(),
                        weight: 100,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert(
                                "ip_addresses".to_string(),
                                serde_json::json!(["192.168.0.0/16"]),
                            );
                            opts
                        },
                    },
                    Rule {
                        id: Some("deny_rule".to_string()),
                        name: Some("Deny Specific".to_string()),
                        rule_type: "ip_deny".to_string(),
                        weight: 90,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert(
                                "ip_addresses".to_string(),
                                serde_json::json!(["192.168.1.0/24"]),
                            );
                            opts
                        },
                    },
                ],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("192.168.1.100"); // Matches both allow and deny
        let result = middleware.left(envelope).await;

        // Should return Err (request denied)
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_no_allow_implicit_deny() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec![Rule {
                    id: Some("allow_rule".to_string()),
                    name: Some("Allow Internal".to_string()),
                    rule_type: "ip_allow".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "ip_addresses".to_string(),
                            serde_json::json!(["192.168.1.0/24"]),
                        );
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("10.0.0.1"); // Does not match allow rule
        let result = middleware.left(envelope).await;

        // Should return Err (implicit deny)
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_allow_all_accepts() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec![Rule {
                    id: Some("allow_all".to_string()),
                    name: Some("Allow All".to_string()),
                    rule_type: "allow_all".to_string(),
                    weight: 100,
                    enabled: true,
                    options: HashMap::new(),
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("any.ip.address");
        let result = middleware.left(envelope).await.unwrap();

        // Should not set skip_backends (request allowed)
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_deny_all_denies() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec![Rule {
                    id: Some("deny_all".to_string()),
                    name: Some("Deny All".to_string()),
                    rule_type: "deny_all".to_string(),
                    weight: 100,
                    enabled: true,
                    options: HashMap::new(),
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("any.ip.address");
        let result = middleware.left(envelope).await;

        // Should return Err (request denied)
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_config_valid() {
        // Create all_policies HashMap with v1.7.0 structure
        let mut all_policies = HashMap::new();
        all_policies.insert(
            "policy1".to_string(),
            crate::config::config::PolicyDefinition {
                id: "policy1".to_string(),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec!["rule1".to_string()],
            },
        );

        // Create all_rules HashMap with v1.7.0 structure
        let mut all_rules = HashMap::new();
        let mut rule1_options = HashMap::new();
        rule1_options.insert(
            "ip_addresses".to_string(),
            serde_json::json!(["192.168.1.0/24"]),
        );
        all_rules.insert(
            "rule1".to_string(),
            crate::config::config::RuleDefinition {
                id: "rule1".to_string(),
                name: Some("Allow Internal".to_string()),
                rule_type: "ip_allow".to_string(),
                weight: 100,
                enabled: true,
                options: rule1_options,
            },
        );

        let mut options = HashMap::new();
        options.insert("policies".to_string(), serde_json::json!(["policy1"]));

        let config = parse_config(&options, &all_policies, &all_rules).unwrap();
        assert_eq!(config.policies.len(), 1);
        assert_eq!(config.policies[0].rules.len(), 1);
        assert_eq!(config.policies[0].rules[0].rule_type, "ip_allow");
    }

    #[test]
    fn test_parse_config_missing_policies() {
        let all_policies = HashMap::new();
        let all_rules = HashMap::new();
        let options = HashMap::new();
        let result = parse_config(&options, &all_policies, &all_rules);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required 'policies'"));
    }

    #[test]
    fn test_parse_config_empty_policies() {
        let all_policies = HashMap::new();
        let all_rules = HashMap::new();
        let mut options = HashMap::new();
        options.insert("policies".to_string(), serde_json::json!([]));
        let result = parse_config(&options, &all_policies, &all_rules);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn test_rate_limit_within_limit_allows() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec![
                    Rule {
                        id: Some("allow_rule".to_string()),
                        name: Some("Allow All".to_string()),
                        rule_type: "allow_all".to_string(),
                        weight: 100,
                        enabled: true,
                        options: HashMap::new(),
                    },
                    Rule {
                        id: Some("rate_limit".to_string()),
                        name: Some("Rate Limit".to_string()),
                        rule_type: "rate_limit".to_string(),
                        weight: 50,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert("max_requests".to_string(), serde_json::json!(5));
                            opts.insert("window_seconds".to_string(), serde_json::json!(60));
                            opts
                        },
                    },
                ],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Send 4 requests - should all be allowed
        for i in 1..=4 {
            let envelope = create_test_envelope("10.0.0.1");
            let result = middleware.left(envelope).await.unwrap();
            assert!(
                !result
                    .request_details
                    .metadata
                    .contains_key("skip_backends"),
                "Request {} should be allowed",
                i
            );
        }
    }

    #[tokio::test]
    async fn test_rate_limit_exceeded_denies() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec![
                    Rule {
                        id: Some("allow_rule".to_string()),
                        name: Some("Allow All".to_string()),
                        rule_type: "allow_all".to_string(),
                        weight: 100,
                        enabled: true,
                        options: HashMap::new(),
                    },
                    Rule {
                        id: Some("rate_limit".to_string()),
                        name: Some("Rate Limit".to_string()),
                        rule_type: "rate_limit".to_string(),
                        weight: 50,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert("max_requests".to_string(), serde_json::json!(3));
                            opts.insert("window_seconds".to_string(), serde_json::json!(60));
                            opts
                        },
                    },
                ],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Send 3 requests - should all be allowed
        for i in 1..=3 {
            let envelope = create_test_envelope("10.0.0.2");
            let result = middleware.left(envelope).await.unwrap();
            assert!(
                !result
                    .request_details
                    .metadata
                    .contains_key("skip_backends"),
                "Request {} should be allowed",
                i
            );
        }

        // 4th request should be denied (rate limit exceeded)
        let envelope = create_test_envelope("10.0.0.2");
        let result = middleware.left(envelope).await;
        assert!(
            result.is_err(),
            "4th request should be denied due to rate limit"
        );
    }

    #[tokio::test]
    async fn test_rate_limit_different_ips_separate_limits() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: Some("Test Policy".to_string()),
                enabled: true,
                rules: vec![
                    Rule {
                        id: Some("allow_rule".to_string()),
                        name: Some("Allow All".to_string()),
                        rule_type: "allow_all".to_string(),
                        weight: 100,
                        enabled: true,
                        options: HashMap::new(),
                    },
                    Rule {
                        id: Some("rate_limit".to_string()),
                        name: Some("Rate Limit".to_string()),
                        rule_type: "rate_limit".to_string(),
                        weight: 50,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert("max_requests".to_string(), serde_json::json!(2));
                            opts.insert("window_seconds".to_string(), serde_json::json!(60));
                            opts
                        },
                    },
                ],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Send 2 requests from IP1 - should be allowed
        for _ in 1..=2 {
            let envelope = create_test_envelope("10.0.0.10");
            let result = middleware.left(envelope).await.unwrap();
            assert!(!result
                .request_details
                .metadata
                .contains_key("skip_backends"));
        }

        // Send 2 requests from IP2 - should also be allowed (separate limit)
        for _ in 1..=2 {
            let envelope = create_test_envelope("10.0.0.20");
            let result = middleware.left(envelope).await.unwrap();
            assert!(!result
                .request_details
                .metadata
                .contains_key("skip_backends"));
        }

        // 3rd request from IP1 should be denied
        let envelope = create_test_envelope("10.0.0.10");
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "3rd request should be denied");
    }

    /// Two `PoliciesMiddleware` instances built with the same instance_key must
    /// share their rate-limit bucket counts. Simulates the executor rebuilding
    /// the middleware on every request.
    #[tokio::test]
    async fn test_rate_limit_shared_across_instances_with_same_key() {
        clear_rate_limit_registry_for_tests();

        fn build_config() -> PoliciesConfig {
            PoliciesConfig {
                policies: vec![Policy {
                    id: Some("test_policy".to_string()),
                    name: Some("Test Policy".to_string()),
                    enabled: true,
                    rules: vec![
                        Rule {
                            id: Some("allow_rule".to_string()),
                            name: Some("Allow All".to_string()),
                            rule_type: "allow_all".to_string(),
                            weight: 100,
                            enabled: true,
                            options: HashMap::new(),
                        },
                        Rule {
                            id: Some("rate_limit".to_string()),
                            name: Some("Rate Limit".to_string()),
                            rule_type: "rate_limit".to_string(),
                            weight: 50,
                            enabled: true,
                            options: {
                                let mut opts = HashMap::new();
                                opts.insert("max_requests".to_string(), serde_json::json!(3));
                                opts.insert("window_seconds".to_string(), serde_json::json!(60));
                                opts
                            },
                        },
                    ],
                }],
            }
        }

        let key = "shared-key-test";

        // First instance records two requests from 10.0.0.42.
        let mw_a = PoliciesMiddleware::with_instance_key(build_config(), key).unwrap();
        for _ in 0..2 {
            let env = create_test_envelope("10.0.0.42");
            mw_a.left(env).await.unwrap();
        }

        // Second instance (fresh construction, same instance_key) sees the
        // running count. Third request from the same IP is allowed, fourth is
        // denied by the shared bucket.
        let mw_b = PoliciesMiddleware::with_instance_key(build_config(), key).unwrap();

        let env = create_test_envelope("10.0.0.42");
        mw_b.left(env).await.unwrap();

        let env = create_test_envelope("10.0.0.42");
        let result = mw_b.left(env).await;
        assert!(
            result.is_err(),
            "4th request across two instances with the same key must be rate-limited"
        );

        // Instances built with a different key do not inherit the bucket.
        let mw_c =
            PoliciesMiddleware::with_instance_key(build_config(), "different-key").unwrap();
        let env = create_test_envelope("10.0.0.42");
        mw_c.left(env).await.expect(
            "different instance_key must not share state with the previous key",
        );
    }

    /// The registry must spawn exactly one cleanup task per instance key. The
    /// task is not directly observable, but we can assert the shared state is
    /// the same `Arc` across instances built with the same key.
    #[tokio::test]
    async fn test_rate_limit_registry_reuses_state_for_same_key() {
        clear_rate_limit_registry_for_tests();

        let key = "single-task-key";
        let cfg = PoliciesConfig {
            policies: vec![Policy {
                id: Some("p".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "allow_all".to_string(),
                    weight: 100,
                    enabled: true,
                    options: HashMap::new(),
                }],
            }],
        };

        let a = PoliciesMiddleware::with_instance_key(cfg.clone(), key).unwrap();
        let b = PoliciesMiddleware::with_instance_key(cfg, key).unwrap();

        assert!(
            Arc::ptr_eq(&a.rate_limit_state, &b.rate_limit_state),
            "Instances with the same key must share the same bucket map Arc"
        );
    }

    #[tokio::test]
    async fn test_time_based_rule_always_allow() {
        // Test time-based rule with allow_during_window=true and no time restrictions
        // This ensures the rule evaluates to allow when all time windows match
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("time_policy".to_string()),
                name: Some("Time Policy".to_string()),
                enabled: true,
                rules: vec![Rule {
                    id: Some("time_rule".to_string()),
                    name: Some("Allow anytime".to_string()),
                    rule_type: "time_based".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert("allow_during_window".to_string(), serde_json::json!(true));
                        opts.insert("timezone".to_string(), serde_json::json!("UTC"));
                        // No time restrictions - always allows
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("10.0.0.1");
        let result = middleware.left(envelope).await.unwrap();

        // Should be allowed (time-based rule allows)
        assert!(
            !result
                .request_details
                .metadata
                .contains_key("skip_backends"),
            "Request should be allowed by time-based rule"
        );
    }

    // ============================================
    // Comprehensive IP Rule Tests
    // ============================================

    #[tokio::test]
    async fn test_ip_allow_ipv6() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test_policy".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "ip_allow".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "ip_addresses".to_string(),
                            serde_json::json!(["2001:db8::/32"]),
                        );
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Create envelope with IPv6 address
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("remote_addr", "2001:db8::1")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();

        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_ip_deny_multiple_ranges() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![
                    Rule {
                        id: None,
                        name: None,
                        rule_type: "allow_all".to_string(),
                        weight: 100,
                        enabled: true,
                        options: HashMap::new(),
                    },
                    Rule {
                        id: None,
                        name: None,
                        rule_type: "ip_deny".to_string(),
                        weight: 90,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert(
                                "ip_addresses".to_string(),
                                serde_json::json!([
                                    "10.0.0.0/8",
                                    "172.16.0.0/12",
                                    "192.168.0.0/16"
                                ]),
                            );
                            opts
                        },
                    },
                ],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test each range
        for ip in ["10.1.1.1", "172.16.5.5", "192.168.100.100"] {
            let envelope = create_test_envelope(ip);
            let result = middleware.left(envelope).await;
            assert!(result.is_err(), "IP {} should be denied", ip);
        }

        // Test non-matching IP
        let envelope = create_test_envelope("8.8.8.8");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_ip_rule_no_metadata() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "ip_allow".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "ip_addresses".to_string(),
                            serde_json::json!(["10.0.0.0/8"]),
                        );
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Create envelope without remote_addr metadata
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();

        let result = middleware.left(envelope).await;
        // Should implicitly deny (no allow matched)
        assert!(result.is_err());
    }

    // ============================================
    // Path Rule Tests
    // ============================================

    #[tokio::test]
    async fn test_path_rule_multiple_patterns() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "path".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "paths".to_string(),
                            serde_json::json!(["/api/public/{*path}", "/health", "/status"]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test matching paths
        for path in ["/api/public/users", "/health", "/status"] {
            let envelope = RequestEnvelope::builder()
                .method("GET")
                .uri("/test")
                .metadata_entry("path", path)
                .original_data(serde_json::Value::Null)
                .normalized_data(Some(serde_json::Value::Null))
                .build()
                .unwrap();

            let result = middleware.left(envelope).await.unwrap();
            assert!(
                !result
                    .request_details
                    .metadata
                    .contains_key("skip_backends"),
                "Path {} should be allowed",
                path
            );
        }

        // Test non-matching path
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("path", "/api/private/users")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();

        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "Non-matching path should be denied");
    }

    // ============================================
    // Geo Rule Tests
    // ============================================

    #[tokio::test]
    async fn test_geo_rule_case_insensitive() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "geo".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "country_codes".to_string(),
                            serde_json::json!(["US", "GB", "au"]), // Mixed case
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test with different case
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("geo_country", "us") // lowercase
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();

        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    // ============================================
    // Time-Based Rule Tests
    // ============================================

    #[tokio::test]
    async fn test_time_based_invalid_timezone() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "time_based".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert("allow_during_window".to_string(), serde_json::json!(true));
                        opts.insert(
                            "timezone".to_string(),
                            serde_json::json!("Invalid/Timezone"),
                        );
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("10.0.0.1");
        let result = middleware.left(envelope).await.unwrap();

        // Should still work (falls back to UTC)
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_time_based_deny_during_window() {
        // Test with allow_during_window=false (maintenance window)
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "time_based".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert("allow_during_window".to_string(), serde_json::json!(false));
                        opts.insert("timezone".to_string(), serde_json::json!("UTC"));
                        // No time restrictions means we're "in the window", so this denies
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("10.0.0.1");
        let result = middleware.left(envelope).await;

        // Should deny (in window + allow_during_window=false = deny)
        assert!(result.is_err());
    }

    // ============================================
    // Disabled Rules/Policies Tests
    // ============================================

    #[tokio::test]
    async fn test_disabled_rule_skipped() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![
                    Rule {
                        id: None,
                        name: None,
                        rule_type: "deny_all".to_string(),
                        weight: 100,
                        enabled: false, // Disabled
                        options: HashMap::new(),
                    },
                    Rule {
                        id: None,
                        name: None,
                        rule_type: "allow_all".to_string(),
                        weight: 50,
                        enabled: true,
                        options: HashMap::new(),
                    },
                ],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("10.0.0.1");
        let result = middleware.left(envelope).await.unwrap();

        // Should allow (deny_all is disabled)
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    // ============================================
    // Method Rule Tests
    // ============================================

    #[tokio::test]
    async fn test_method_rule_allow_mode() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "method".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert("methods".to_string(), serde_json::json!(["GET", "POST"]));
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test GET - should allow
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test POST - should allow
        let envelope = RequestEnvelope::builder()
            .method("POST")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test DELETE - should deny (implicit deny, no allow matched)
        let envelope = RequestEnvelope::builder()
            .method("DELETE")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "DELETE should be denied");
    }

    #[tokio::test]
    async fn test_method_rule_deny_mode() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![
                    Rule {
                        id: None,
                        name: None,
                        rule_type: "allow_all".to_string(),
                        weight: 100,
                        enabled: true,
                        options: HashMap::new(),
                    },
                    Rule {
                        id: None,
                        name: None,
                        rule_type: "method".to_string(),
                        weight: 90,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert("methods".to_string(), serde_json::json!(["DELETE"]));
                            opts.insert("mode".to_string(), serde_json::json!("deny"));
                            opts
                        },
                    },
                ],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test GET - should allow (not in deny list)
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test DELETE - should deny
        let envelope = RequestEnvelope::builder()
            .method("DELETE")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "DELETE should be denied");
    }

    #[tokio::test]
    async fn test_method_rule_case_insensitive() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "method".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "methods".to_string(),
                            serde_json::json!(["get", "post"]), // lowercase
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test with uppercase method
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    // ============================================
    // User-Agent Rule Tests
    // ============================================

    #[tokio::test]
    async fn test_user_agent_rule_deny_bots() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![
                    Rule {
                        id: None,
                        name: None,
                        rule_type: "allow_all".to_string(),
                        weight: 100,
                        enabled: true,
                        options: HashMap::new(),
                    },
                    Rule {
                        id: None,
                        name: None,
                        rule_type: "user_agent".to_string(),
                        weight: 90,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert(
                                "patterns".to_string(),
                                serde_json::json!([
                                    {
                                        "label": "Common Bots",
                                        "pattern": "/bot|crawler|spider/i"
                                    },
                                    {
                                        "label": "Google Bot",
                                        "pattern": "/googlebot/i"
                                    }
                                ]),
                            );
                            opts.insert("mode".to_string(), serde_json::json!("deny"));
                            opts
                        },
                    },
                ],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test bot User-Agent - should deny
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .header("user-agent", "Mozilla/5.0 (compatible; Googlebot/2.1")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "Bot user-agent should be denied");

        // Test regular User-Agent - should allow
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_user_agent_rule_missing_header() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "user_agent".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "patterns".to_string(),
                            serde_json::json!([{"label": "Bot", "pattern": "/bot/i"}]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("deny"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test without User-Agent header - should not match (implicit deny)
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(
            result.is_err(),
            "Missing user-agent should result in implicit deny"
        );
    }

    // ============================================
    // Content-Type Rule Tests
    // ============================================

    #[tokio::test]
    async fn test_content_type_rule_exact_match() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "content_type".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "content_types".to_string(),
                            serde_json::json!([
                                "application/json",
                                "application/x-www-form-urlencoded"
                            ]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test matching content type
        let envelope = RequestEnvelope::builder()
            .method("POST")
            .uri("/test")
            .header("content-type", "application/json")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_content_type_rule_wildcard() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "content_type".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "content_types".to_string(),
                            serde_json::json!(["application/*"]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test wildcard match for application/json
        let envelope = RequestEnvelope::builder()
            .method("POST")
            .uri("/test")
            .header("content-type", "application/json")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test wildcard match for application/xml
        let envelope = RequestEnvelope::builder()
            .method("POST")
            .uri("/test")
            .header("content-type", "application/xml")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test non-matching content type
        let envelope = RequestEnvelope::builder()
            .method("POST")
            .uri("/test")
            .header("content-type", "text/html")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "text/html should be denied");
    }

    #[tokio::test]
    async fn test_content_type_rule_charset_handling() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "content_type".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "content_types".to_string(),
                            serde_json::json!(["application/json"]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test with charset parameter
        let envelope = RequestEnvelope::builder()
            .method("POST")
            .uri("/test")
            .header("content-type", "application/json; charset=utf-8")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(
            !result
                .request_details
                .metadata
                .contains_key("skip_backends"),
            "Content-Type with charset should still match"
        );
    }

    // ============================================
    // Query Parameter Rule Tests
    // ============================================

    #[tokio::test]
    async fn test_query_parameter_rule_exists() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "query_parameter".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "parameters".to_string(),
                            serde_json::json!([
                                {
                                    "name": "api_key",
                                    "match_type": "exists",
                                    "value": ""
                                }
                            ]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test with parameter present
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("param_api_key", "abc123")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test without parameter
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "Missing api_key should be denied");
    }

    #[tokio::test]
    async fn test_query_parameter_rule_exact_match() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "query_parameter".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "parameters".to_string(),
                            serde_json::json!([
                                {
                                    "name": "version",
                                    "match_type": "exact",
                                    "value": "v2"
                                }
                            ]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test exact match
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("param_version", "v2")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test non-matching value
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("param_version", "v1")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "Non-matching version should be denied");
    }

    #[tokio::test]
    async fn test_query_parameter_rule_regex() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "query_parameter".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "parameters".to_string(),
                            serde_json::json!([
                                {
                                    "name": "id",
                                    "match_type": "regex",
                                    "value": "^[0-9]+$"
                                }
                            ]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test matching regex
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("param_id", "42")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test non-matching regex
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("param_id", "abc")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "Non-numeric id should be denied");
    }

    #[tokio::test]
    async fn test_query_parameter_rule_multiple_and_logic() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "query_parameter".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "parameters".to_string(),
                            serde_json::json!([
                                {
                                    "name": "api_key",
                                    "match_type": "exists",
                                    "value": ""
                                },
                                {
                                    "name": "action",
                                    "match_type": "exact",
                                    "value": "query"
                                }
                            ]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Test both parameters match
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("param_api_key", "abc123")
            .metadata_entry("param_action", "query")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Test first parameter missing
        let envelope = RequestEnvelope::builder()
            .method("GET")
            .uri("/test")
            .metadata_entry("param_action", "query")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap();
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "Missing api_key should be denied");
    }

    #[tokio::test]
    async fn test_multiple_policies_interaction() {
        let config = PoliciesConfig {
            policies: vec![
                Policy {
                    id: Some("policy1".to_string()),
                    name: None,
                    enabled: true,
                    rules: vec![Rule {
                        id: None,
                        name: None,
                        rule_type: "ip_allow".to_string(),
                        weight: 100,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert(
                                "ip_addresses".to_string(),
                                serde_json::json!(["192.168.0.0/16"]),
                            );
                            opts
                        },
                    }],
                },
                Policy {
                    id: Some("policy2".to_string()),
                    name: None,
                    enabled: true,
                    rules: vec![Rule {
                        id: None,
                        name: None,
                        rule_type: "ip_deny".to_string(),
                        weight: 90,
                        enabled: true,
                        options: {
                            let mut opts = HashMap::new();
                            opts.insert(
                                "ip_addresses".to_string(),
                                serde_json::json!(["192.168.1.0/24"]),
                            );
                            opts
                        },
                    }],
                },
            ],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();

        // IP matches allow in policy1 and deny in policy2 -> deny
        let envelope = create_test_envelope("192.168.1.100");
        let result = middleware.left(envelope).await;
        assert!(result.is_err(), "IP matches deny rule, should be denied");

        // IP matches allow in policy1 but not deny in policy2 -> allow
        let envelope = create_test_envelope("192.168.2.100");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    // ============================================
    // IP Allowlist Gate Tests (AURA-2438)
    // ============================================

    fn ip_rule(id: &str, rule_type: &str, cidrs: &[&str]) -> Rule {
        let mut options = HashMap::new();
        options.insert("ip_addresses".to_string(), serde_json::json!(cidrs));
        Rule {
            id: Some(id.to_string()),
            name: None,
            rule_type: rule_type.to_string(),
            weight: 90,
            enabled: true,
            options,
        }
    }

    fn method_rule(id: &str, mode: &str, methods: &[&str]) -> Rule {
        let mut options = HashMap::new();
        options.insert("mode".to_string(), serde_json::json!(mode));
        options.insert("methods".to_string(), serde_json::json!(methods));
        Rule {
            id: Some(id.to_string()),
            name: None,
            rule_type: "method".to_string(),
            weight: 80,
            enabled: true,
            options,
        }
    }

    fn single_policy(rules: Vec<Rule>) -> PoliciesConfig {
        PoliciesConfig {
            policies: vec![Policy {
                id: Some("gate_policy".to_string()),
                name: None,
                enabled: true,
                rules,
            }],
        }
    }

    /// Envelope with an optional injected client IP under `remote_addr`.
    fn gate_envelope(method: &str, client_ip: Option<&str>) -> RequestEnvelope<serde_json::Value> {
        let mut builder = RequestEnvelopeBuilder::new()
            .method(method)
            .uri("/api/items")
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null));
        if let Some(ip) = client_ip {
            builder = builder.metadata_entry("remote_addr", ip);
        }
        builder.build().unwrap()
    }

    /// The shape of the harmony-examples `http-internal` policy: an allowlist
    /// plus a method rule in allow mode that allows the common methods from
    /// any address.
    fn allowlist_with_method_allow() -> PoliciesConfig {
        single_policy(vec![
            ip_rule(
                "internal_networks",
                "ip_allow",
                &["10.0.0.0/8", "192.168.0.0/16", "127.0.0.1/32"],
            ),
            method_rule("method_filter", "allow", &["GET", "POST", "PUT", "DELETE"]),
        ])
    }

    fn assert_access_denied(result: Result<RequestEnvelope<serde_json::Value>, Error>) {
        let err = result.expect_err("request should be denied");
        assert!(
            err.downcast_ref::<AccessDenied>().is_some(),
            "expected AccessDenied (403), got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_ip_allowlist_gate_denies_outside_ip_even_when_method_allows() {
        let middleware = PoliciesMiddleware::new(allowlist_with_method_allow()).unwrap();

        let result = middleware
            .left(gate_envelope("GET", Some("203.0.113.9")))
            .await;

        assert_access_denied(result);
    }

    #[tokio::test]
    async fn test_ip_allowlist_gate_admits_inside_ip_with_allowed_method() {
        let middleware = PoliciesMiddleware::new(allowlist_with_method_allow()).unwrap();

        for ip in ["10.1.2.3", "192.168.7.7", "127.0.0.1"] {
            let result = middleware.left(gate_envelope("GET", Some(ip))).await;
            assert!(
                result.is_ok(),
                "IP {} is in the allowlist and should pass",
                ip
            );
        }
    }

    #[tokio::test]
    async fn test_ip_allowlist_gate_inside_ip_still_subject_to_method_rules() {
        // Allow-mode method rule: a method outside its list is denied.
        let middleware = PoliciesMiddleware::new(allowlist_with_method_allow()).unwrap();
        let err = middleware
            .left(gate_envelope("PATCH", Some("10.1.2.3")))
            .await
            .expect_err("PATCH is not in the allowed methods");
        assert!(err.downcast_ref::<MethodDenied>().is_some(), "got: {}", err);

        // Deny-mode method rule: a listed method is denied.
        let middleware = PoliciesMiddleware::new(single_policy(vec![
            ip_rule("internal_networks", "ip_allow", &["10.0.0.0/8"]),
            method_rule("no_delete", "deny", &["DELETE"]),
        ]))
        .unwrap();
        let err = middleware
            .left(gate_envelope("DELETE", Some("10.1.2.3")))
            .await
            .expect_err("DELETE is denied by the method rule");
        assert!(err.downcast_ref::<MethodDenied>().is_some(), "got: {}", err);
        assert!(middleware
            .left(gate_envelope("GET", Some("10.1.2.3")))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_ip_allowlist_gate_fails_closed_on_unknown_client_ip() {
        let middleware = PoliciesMiddleware::new(allowlist_with_method_allow()).unwrap();

        // No remote_addr at all (for example an HTTP/3 request).
        assert_access_denied(middleware.left(gate_envelope("GET", None)).await);

        // A remote_addr that does not parse as an IP address.
        assert_access_denied(
            middleware
                .left(gate_envelope("GET", Some("not-an-ip")))
                .await,
        );
    }

    #[tokio::test]
    async fn test_no_ip_allowlist_keeps_any_allow_semantics() {
        let middleware = PoliciesMiddleware::new(single_policy(vec![method_rule(
            "method_filter",
            "allow",
            &["GET"],
        )]))
        .unwrap();

        // Any address, and no address at all, is admitted by the method rule.
        assert!(middleware
            .left(gate_envelope("GET", Some("203.0.113.9")))
            .await
            .is_ok());
        assert!(middleware.left(gate_envelope("GET", None)).await.is_ok());
    }

    #[tokio::test]
    async fn test_ip_deny_rule_still_denies_inside_allowlist() {
        let middleware = PoliciesMiddleware::new(single_policy(vec![
            ip_rule("internal_networks", "ip_allow", &["10.0.0.0/8"]),
            ip_rule("quarantine", "ip_deny", &["10.9.0.0/16"]),
            method_rule("method_filter", "allow", &["GET"]),
        ]))
        .unwrap();

        assert_access_denied(
            middleware
                .left(gate_envelope("GET", Some("10.9.1.1")))
                .await,
        );
        assert!(middleware
            .left(gate_envelope("GET", Some("10.8.1.1")))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_ip_deny_rule_without_allowlist_unchanged() {
        let middleware = PoliciesMiddleware::new(single_policy(vec![
            ip_rule("blocklist", "ip_deny", &["203.0.113.0/24"]),
            method_rule("method_filter", "allow", &["GET"]),
        ]))
        .unwrap();

        assert_access_denied(
            middleware
                .left(gate_envelope("GET", Some("203.0.113.9")))
                .await,
        );
        assert!(middleware
            .left(gate_envelope("GET", Some("198.51.100.1")))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_several_ip_allow_rules_in_one_policy_form_a_union() {
        let middleware = PoliciesMiddleware::new(single_policy(vec![
            ip_rule("office", "ip_allow", &["192.168.0.0/16"]),
            ip_rule("vpn", "ip_allow", &["10.0.0.0/8"]),
            method_rule("method_filter", "allow", &["GET"]),
        ]))
        .unwrap();

        assert!(middleware
            .left(gate_envelope("GET", Some("192.168.1.1")))
            .await
            .is_ok());
        assert!(middleware
            .left(gate_envelope("GET", Some("10.1.1.1")))
            .await
            .is_ok());
        assert_access_denied(middleware.left(gate_envelope("GET", Some("8.8.8.8"))).await);
    }

    #[tokio::test]
    async fn test_ip_allowlists_in_separate_policies_form_a_union() {
        // "office OR VPN" as two policies, each with an allowlist and a method
        // rule, so a request that passes the gate still has an allow rule.
        let config = PoliciesConfig {
            policies: vec![
                Policy {
                    id: Some("office".to_string()),
                    name: None,
                    enabled: true,
                    rules: vec![
                        ip_rule("office_nets", "ip_allow", &["10.0.0.0/8"]),
                        method_rule("office_methods", "allow", &["GET"]),
                    ],
                },
                Policy {
                    id: Some("vpn".to_string()),
                    name: None,
                    enabled: true,
                    rules: vec![
                        ip_rule("vpn_nets", "ip_allow", &["10.1.0.0/16", "172.16.0.0/12"]),
                        method_rule("vpn_methods", "allow", &["GET"]),
                    ],
                },
            ],
        };
        let middleware = PoliciesMiddleware::new(config).unwrap();

        // Inside both allowlists.
        assert!(middleware
            .left(gate_envelope("GET", Some("10.1.2.3")))
            .await
            .is_ok());
        // Inside the office allowlist only.
        assert!(middleware
            .left(gate_envelope("GET", Some("10.2.2.3")))
            .await
            .is_ok());
        // Inside the VPN allowlist only.
        assert!(middleware
            .left(gate_envelope("GET", Some("172.16.5.5")))
            .await
            .is_ok());
        // Inside neither allowlist.
        assert_access_denied(
            middleware
                .left(gate_envelope("GET", Some("203.0.113.9")))
                .await,
        );
    }

    #[tokio::test]
    async fn test_ip_allowlist_in_one_policy_gates_the_whole_middleware() {
        // The second policy has no allowlist, but its method rule does not
        // bypass the first policy's allowlist.
        let config = PoliciesConfig {
            policies: vec![
                Policy {
                    id: Some("gated".to_string()),
                    name: None,
                    enabled: true,
                    rules: vec![ip_rule("internal_networks", "ip_allow", &["10.0.0.0/8"])],
                },
                Policy {
                    id: Some("methods".to_string()),
                    name: None,
                    enabled: true,
                    rules: vec![method_rule("method_filter", "allow", &["GET"])],
                },
            ],
        };
        let middleware = PoliciesMiddleware::new(config).unwrap();

        assert!(middleware
            .left(gate_envelope("GET", Some("10.2.2.3")))
            .await
            .is_ok());
        assert_access_denied(
            middleware
                .left(gate_envelope("GET", Some("203.0.113.9")))
                .await,
        );
        assert_access_denied(middleware.left(gate_envelope("GET", None)).await);
    }

    #[tokio::test]
    async fn test_disabled_ip_allow_rule_does_not_gate() {
        let mut disabled = ip_rule("internal_networks", "ip_allow", &["10.0.0.0/8"]);
        disabled.enabled = false;
        let middleware = PoliciesMiddleware::new(single_policy(vec![
            disabled,
            method_rule("method_filter", "allow", &["GET"]),
        ]))
        .unwrap();

        assert!(middleware
            .left(gate_envelope("GET", Some("203.0.113.9")))
            .await
            .is_ok());
    }

    // ============================================
    // Configuration Validation Tests
    // ============================================

    #[test]
    fn test_invalid_ip_cidr() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "ip_allow".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "ip_addresses".to_string(),
                            serde_json::json!(["not.an.ip.address"]),
                        );
                        opts
                    },
                }],
            }],
        };

        let result = PoliciesMiddleware::new(config);
        assert!(result.is_err());
        let err_msg = result.err().unwrap();
        assert!(
            err_msg.contains("Invalid IP address"),
            "Error was: {}",
            err_msg
        );
    }

    #[test]
    fn test_invalid_path_pattern() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "path".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert(
                            "paths".to_string(),
                            serde_json::json!(["no-leading-slash"]), // Invalid: no /
                        );
                        opts.insert("mode".to_string(), serde_json::json!("allow"));
                        opts
                    },
                }],
            }],
        };

        let result = PoliciesMiddleware::new(config);
        assert!(result.is_err());
        let err_msg = result.err().unwrap();
        assert!(
            err_msg.contains("must start with '/'"),
            "Error was: {}",
            err_msg
        );
    }

    #[test]
    fn test_invalid_path_mode() {
        let config = PoliciesConfig {
            policies: vec![Policy {
                id: Some("test".to_string()),
                name: None,
                enabled: true,
                rules: vec![Rule {
                    id: None,
                    name: None,
                    rule_type: "path".to_string(),
                    weight: 100,
                    enabled: true,
                    options: {
                        let mut opts = HashMap::new();
                        opts.insert("paths".to_string(), serde_json::json!(["/api/{*path}"]));
                        opts.insert("mode".to_string(), serde_json::json!("invalid"));
                        opts
                    },
                }],
            }],
        };

        let result = PoliciesMiddleware::new(config);
        assert!(result.is_err());
        let err_msg = result.err().unwrap();
        assert!(
            err_msg.contains("must be 'allow' or 'deny'"),
            "Error was: {}",
            err_msg
        );
    }

    // ============================================
    // IPv4-mapped IPv6 client addresses
    // ============================================
    //
    // A dual-stack listener (bind "::", IPV6_V6ONLY=0) reports an IPv4 client
    // as ::ffff:a.b.c.d. These tests check that such a client is treated as
    // the IPv4 address it carries.

    #[tokio::test]
    async fn test_ipv4_mapped_client_matches_ipv4_allowlist() {
        let middleware = PoliciesMiddleware::new(allowlist_with_method_allow()).unwrap();

        assert!(
            middleware
                .left(gate_envelope("GET", Some("::ffff:10.1.2.3")))
                .await
                .is_ok(),
            "::ffff:10.1.2.3 is 10.1.2.3 and is inside 10.0.0.0/8"
        );
        assert_access_denied(
            middleware
                .left(gate_envelope("GET", Some("::ffff:203.0.113.9")))
                .await,
        );
    }

    #[tokio::test]
    async fn test_ipv4_mapped_client_matches_ipv4_deny() {
        let blocklist_only = PoliciesMiddleware::new(single_policy(vec![
            ip_rule("blocklist", "ip_deny", &["203.0.113.0/24"]),
            method_rule("method_filter", "allow", &["GET"]),
        ]))
        .unwrap();
        assert_access_denied(
            blocklist_only
                .left(gate_envelope("GET", Some("::ffff:203.0.113.5")))
                .await,
        );
        assert!(blocklist_only
            .left(gate_envelope("GET", Some("::ffff:198.51.100.5")))
            .await
            .is_ok());

        let deny_inside_allowlist = PoliciesMiddleware::new(single_policy(vec![
            ip_rule("internal_networks", "ip_allow", &["10.0.0.0/8"]),
            ip_rule("quarantine", "ip_deny", &["10.9.0.0/16"]),
            method_rule("method_filter", "allow", &["GET"]),
        ]))
        .unwrap();
        // The allowlist admits a mapped client, so the 10.9 denial below comes
        // from the deny rule and not from the gate.
        assert!(deny_inside_allowlist
            .left(gate_envelope("GET", Some("::ffff:10.1.1.1")))
            .await
            .is_ok());
        assert_access_denied(
            deny_inside_allowlist
                .left(gate_envelope("GET", Some("::ffff:10.9.1.1")))
                .await,
        );
    }

    #[test]
    fn test_ip_rule_written_as_ipv6_mapped_network_still_matches() {
        let rule = CompiledIpRule::new(&["::ffff:10.0.0.0/104".to_string()]).unwrap();

        let mapped = IpAddr::from_str("::ffff:10.1.2.3").unwrap();
        assert!(rule.matches(&mapped));

        let ipv4_rule = CompiledIpRule::new(&["10.0.0.0/8".to_string()]).unwrap();
        assert!(ipv4_rule.matches(&mapped));
        assert!(!ipv4_rule.matches(&IpAddr::from_str("2001:db8::1").unwrap()));
    }

    #[test]
    fn test_rate_limit_client_key_folds_ipv4_mapped_address() {
        assert_eq!(rate_limit_client_key("::ffff:10.0.0.10"), "10.0.0.10");
        assert_eq!(rate_limit_client_key("10.0.0.10"), "10.0.0.10");
        assert_eq!(rate_limit_client_key("2001:db8::1"), "2001:db8::1");
        // Parsing also normalises IPv6 text, so spellings of one address share a key.
        assert_eq!(rate_limit_client_key("2001:DB8:0::1"), "2001:db8::1");
        assert_eq!(rate_limit_client_key("not-an-ip"), "not-an-ip");
    }

    #[tokio::test]
    async fn test_rate_limit_ipv4_mapped_and_plain_share_a_bucket() {
        let mut options = HashMap::new();
        options.insert("max_requests".to_string(), serde_json::json!(2));
        options.insert("window_seconds".to_string(), serde_json::json!(60));
        let middleware = PoliciesMiddleware::new(single_policy(vec![
            Rule {
                id: Some("allow_rule".to_string()),
                name: None,
                rule_type: "allow_all".to_string(),
                weight: 100,
                enabled: true,
                options: HashMap::new(),
            },
            Rule {
                id: Some("rate_limit".to_string()),
                name: None,
                rule_type: "rate_limit".to_string(),
                weight: 50,
                enabled: true,
                options,
            },
        ]))
        .unwrap();

        assert!(middleware
            .left(create_test_envelope("10.0.0.10"))
            .await
            .is_ok());
        assert!(middleware
            .left(create_test_envelope("::ffff:10.0.0.10"))
            .await
            .is_ok());

        // Both forms counted against one bucket, so the third request is over
        // the limit whichever form it uses.
        assert!(
            middleware
                .left(create_test_envelope("::ffff:10.0.0.10"))
                .await
                .is_err(),
            "mapped and plain forms of one client must share a rate-limit bucket"
        );
        assert!(middleware
            .left(create_test_envelope("10.0.0.10"))
            .await
            .is_err());
    }
}
