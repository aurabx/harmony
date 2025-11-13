use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use chrono::Utc;
use ipnetwork::IpNetwork;
use matchit::Router;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

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

    fn matches(&self, ip: &IpAddr) -> bool {
        self.networks.iter().any(|network| network.contains(*ip))
    }
}

/// Policies middleware implementation
pub struct PoliciesMiddleware {
    policies: Vec<Policy>,
    // Pre-compiled IP rules for performance
    compiled_ip_rules: HashMap<usize, CompiledIpRule>, // policy_idx -> rule_idx key
    // Pre-compiled path rules for performance
    compiled_path_rules: HashMap<usize, CompiledPathRule>,
    // Rate limit state tracking (client_ip + rule_key -> state)
    rate_limit_state: Arc<RwLock<HashMap<String, RateLimitState>>>,
}

impl PoliciesMiddleware {
    pub fn new(config: PoliciesConfig) -> Result<Self, String> {
        if config.policies.is_empty() {
            return Err("Policies middleware requires at least one policy".to_string());
        }

        // Filter to enabled policies only
        let enabled_policies: Vec<Policy> = config
            .policies
            .into_iter()
            .filter(|p| p.enabled)
            .collect();

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
            "PoliciesMiddleware initialized with {} enabled policies",
            enabled_policies.len()
        );

        let rate_limit_state = Arc::new(RwLock::new(HashMap::new()));
        
        // Spawn background task to cleanup expired rate limit entries
        Self::spawn_cleanup_task(rate_limit_state.clone());

        Ok(Self {
            policies: enabled_policies,
            compiled_ip_rules,
            compiled_path_rules,
            rate_limit_state,
        })
    }

    /// Spawn a background task to periodically clean up expired rate limit entries
    fn spawn_cleanup_task(state: Arc<RwLock<HashMap<String, RateLimitState>>>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            
            loop {
                interval.tick().await;
                
                let mut state_map = state.write().await;
                let current_time = Utc::now().timestamp();
                let initial_size = state_map.len();
                
                // Remove entries older than 5 minutes (300 seconds)
                // This is conservative - keeps entries longer than most rate limit windows
                state_map.retain(|_key, state| {
                    current_time - state.window_start < 300
                });
                
                let removed = initial_size - state_map.len();
                if removed > 0 {
                    tracing::debug!(
                        "Rate limit cleanup: removed {} expired entries, {} remaining",
                        removed,
                        state_map.len()
                    );
                }
            }
        });
        
        tracing::info!("Rate limit cleanup task started (runs every 60 seconds)");
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
            "rate_limit" => self.evaluate_rate_limit_rule(rule, policy_idx, rule_idx, envelope).await,
            "path" => self.evaluate_path_rule(rule, policy_idx, rule_idx, envelope),
            "geo" => self.evaluate_geo_rule(rule, envelope),
            "header" => self.evaluate_header_rule(rule, envelope),
            "time_based" => self.evaluate_time_based_rule(rule),
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
            .cloned()
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
                tracing::error!("Compiled path rule not found for key {} - this is a bug", key);
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
                "regex" => {
                    match Regex::new(expected_value) {
                        Ok(re) => re.is_match(actual_value),
                        Err(e) => {
                            tracing::warn!(
                                "Invalid regex pattern '{}': {}",
                                expected_value,
                                e
                            );
                            false
                        }
                    }
                }
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
                tracing::warn!(
                    "Invalid timezone '{}': {} - using UTC",
                    timezone_str,
                    e
                );
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
}

#[async_trait]
impl Middleware for PoliciesMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        let mut has_allow = false;
        let mut has_deny = false;
        let mut deny_status = 403; // Default deny status
        let mut deny_reason = "Access denied by policy";

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
                let evaluation = self.evaluate_rule(rule, policy_idx, rule_idx, &envelope).await;

                match evaluation {
                    RuleEvaluation::Allow => {
                        has_allow = true;
                        tracing::debug!(
                            "Rule {:?} matched - ALLOW",
                            rule.name.as_deref().unwrap_or("unnamed")
                        );
                    }
                    RuleEvaluation::Deny => {
                        has_deny = true;
                        // Set appropriate status code based on rule type
                        if rule.rule_type == "rate_limit" {
                            deny_status = 429;
                            deny_reason = "Rate limit exceeded";
                        }
                        tracing::warn!(
                            "Rule {:?} matched - DENY (status: {})",
                            rule.name.as_deref().unwrap_or("unnamed"),
                            deny_status
                        );
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
        // Request is ACCEPTED only if: has_allow = true AND has_deny = false
        if has_deny {
            tracing::warn!("Request DENIED - at least one deny rule matched");
            envelope
                .request_details
                .metadata
                .insert("skip_backends".to_string(), "true".to_string());
            envelope.normalized_data = Some(serde_json::json!({
                "response": {
                    "status": deny_status,
                    "body": deny_reason
                }
            }));
        } else if !has_allow {
            tracing::warn!("Request DENIED - no allow rule matched (implicit deny)");
            envelope
                .request_details
                .metadata
                .insert("skip_backends".to_string(), "true".to_string());
            envelope.normalized_data = Some(serde_json::json!({
                "response": {
                    "status": 403,
                    "body": "Access denied by policy"
                }
            }));
        } else {
            tracing::debug!("Request ALLOWED - has allow rule(s) and no deny rules");
        }

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
pub fn parse_config(options: &HashMap<String, Value>) -> Result<PoliciesConfig, String> {
    let policies_array = options
        .get("policies")
        .and_then(|v| v.as_array())
        .ok_or("Missing required 'policies' array in policies middleware config")?;

    if policies_array.is_empty() {
        return Err("Policies array cannot be empty".to_string());
    }

    let mut policies = Vec::new();

    for (idx, policy_value) in policies_array.iter().enumerate() {
        let policy_obj = policy_value.as_object().ok_or_else(|| {
            format!("Policy at index {} must be an object", idx)
        })?;

        let id = policy_obj.get("id").and_then(|v| v.as_str()).map(String::from);
        let name = policy_obj.get("name").and_then(|v| v.as_str()).map(String::from);
        let enabled = policy_obj
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let rules_array = policy_obj
            .get("rules")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                format!("Policy at index {} missing required 'rules' array", idx)
            })?;

        let mut rules = Vec::new();

        for (rule_idx, rule_value) in rules_array.iter().enumerate() {
            let rule_obj = rule_value.as_object().ok_or_else(|| {
                format!("Rule at index {} in policy {} must be an object", rule_idx, idx)
            })?;

            let rule_id = rule_obj.get("id").and_then(|v| v.as_str()).map(String::from);
            let rule_name = rule_obj.get("name").and_then(|v| v.as_str()).map(String::from);
            let rule_type = rule_obj
                .get("rule_type")
                .or_else(|| rule_obj.get("type"))  // Support both for compatibility
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    format!(
                        "Rule at index {} in policy {} missing required 'rule_type' field",
                        rule_idx, idx
                    )
                })?
                .to_string();

            let weight = rule_obj
                .get("weight")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let rule_enabled = rule_obj
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let options = rule_obj
                .get("options")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<HashMap<String, Value>>()
                })
                .unwrap_or_default();

            rules.push(Rule {
                id: rule_id,
                name: rule_name,
                rule_type,
                weight,
                enabled: rule_enabled,
                options,
            });
        }

        policies.push(Policy {
            id,
            name,
            enabled,
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
        let result = middleware.left(envelope).await.unwrap();

        // Should set skip_backends (request denied)
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string())
        );

        // Should set 403 response
        let response = result
            .normalized_data
            .as_ref()
            .unwrap()
            .get("response")
            .unwrap();
        assert_eq!(response.get("status").unwrap().as_u64().unwrap(), 403);
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
        let result = middleware.left(envelope).await.unwrap();

        // Should set skip_backends (implicit deny)
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string())
        );

        // Should set 403 response
        let response = result
            .normalized_data
            .as_ref()
            .unwrap()
            .get("response")
            .unwrap();
        assert_eq!(response.get("status").unwrap().as_u64().unwrap(), 403);
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
        let result = middleware.left(envelope).await.unwrap();

        // Should set skip_backends (request denied)
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string())
        );
    }

    #[test]
    fn test_parse_config_valid() {
        let mut options = HashMap::new();
        options.insert(
            "policies".to_string(),
            serde_json::json!([
                {
                    "id": "policy1",
                    "name": "Test Policy",
                    "enabled": true,
                    "rules": [
                        {
                            "id": "rule1",
                            "name": "Allow Internal",
                            "type": "ip_allow",
                            "weight": 100,
                            "enabled": true,
                            "options": {
                                "ip_addresses": ["192.168.1.0/24"]
                            }
                        }
                    ]
                }
            ]),
        );

        let config = parse_config(&options).unwrap();
        assert_eq!(config.policies.len(), 1);
        assert_eq!(config.policies[0].rules.len(), 1);
        assert_eq!(config.policies[0].rules[0].rule_type, "ip_allow");
    }

    #[test]
    fn test_parse_config_missing_policies() {
        let options = HashMap::new();
        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required 'policies'"));
    }

    #[test]
    fn test_parse_config_empty_policies() {
        let mut options = HashMap::new();
        options.insert("policies".to_string(), serde_json::json!([]));
        let result = parse_config(&options);
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
                !result.request_details.metadata.contains_key("skip_backends"),
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
                !result.request_details.metadata.contains_key("skip_backends"),
                "Request {} should be allowed",
                i
            );
        }

        // 4th request should be denied (rate limit exceeded)
        let envelope = create_test_envelope("10.0.0.2");
        let result = middleware.left(envelope).await.unwrap();
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string()),
            "4th request should be denied due to rate limit"
        );

        // Verify 429 response (rate limit uses 429, not 403)
        let response = result
            .normalized_data
            .as_ref()
            .unwrap()
            .get("response")
            .unwrap();
        assert_eq!(response.get("status").unwrap().as_u64().unwrap(), 429);
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
            assert!(!result.request_details.metadata.contains_key("skip_backends"));
        }

        // Send 2 requests from IP2 - should also be allowed (separate limit)
        for _ in 1..=2 {
            let envelope = create_test_envelope("10.0.0.20");
            let result = middleware.left(envelope).await.unwrap();
            assert!(!result.request_details.metadata.contains_key("skip_backends"));
        }

        // 3rd request from IP1 should be denied
        let envelope = create_test_envelope("10.0.0.10");
        let result = middleware.left(envelope).await.unwrap();
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string())
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
            !result.request_details.metadata.contains_key("skip_backends"),
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
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
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
                                serde_json::json!(["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"]),
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
            let result = middleware.left(envelope).await.unwrap();
            assert_eq!(
                result.request_details.metadata.get("skip_backends"),
                Some(&"true".to_string()),
                "IP {} should be denied",
                ip
            );
        }
        
        // Test non-matching IP
        let envelope = create_test_envelope("8.8.8.8");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
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
            
        let result = middleware.left(envelope).await.unwrap();
        // Should implicitly deny (no allow matched)
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string())
        );
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
                !result.request_details.metadata.contains_key("skip_backends"),
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
            
        let result = middleware.left(envelope).await.unwrap();
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string())
        );
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
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
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
                        opts.insert("timezone".to_string(), serde_json::json!("Invalid/Timezone"));
                        opts
                    },
                }],
            }],
        };

        let middleware = PoliciesMiddleware::new(config).unwrap();
        let envelope = create_test_envelope("10.0.0.1");
        let result = middleware.left(envelope).await.unwrap();
        
        // Should still work (falls back to UTC)
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
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
        let result = middleware.left(envelope).await.unwrap();
        
        // Should deny (in window + allow_during_window=false = deny)
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string())
        );
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
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
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
        let result = middleware.left(envelope).await.unwrap();
        assert_eq!(
            result.request_details.metadata.get("skip_backends"),
            Some(&"true".to_string())
        );
        
        // IP matches allow in policy1 but not deny in policy2 -> allow
        let envelope = create_test_envelope("192.168.2.100");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
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
        assert!(err_msg.contains("Invalid IP address"), "Error was: {}", err_msg);
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
        assert!(err_msg.contains("must start with '/'"), "Error was: {}", err_msg);
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
                        opts.insert(
                            "paths".to_string(),
                            serde_json::json!(["/api/{*path}"]),
                        );
                        opts.insert("mode".to_string(), serde_json::json!("invalid"));
                        opts
                    },
                }],
            }],
        };

        let result = PoliciesMiddleware::new(config);
        assert!(result.is_err());
        let err_msg = result.err().unwrap();
        assert!(err_msg.contains("must be 'allow' or 'deny'"), "Error was: {}", err_msg);
    }
}
