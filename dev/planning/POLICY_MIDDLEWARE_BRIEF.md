# Implementation Brief: Policy Middleware Handler

## Overview
Implement a new `policies` middleware type that reads policy definitions and applies rules to incoming requests. This middleware enables flexible access control, rate limiting, and other policy-based request processing.

## Schema Reference (harmony-dsl v1.5.0)
The policy schema supports:
- **Policies array** (`options.policies`): Multiple policy definitions per middleware instance
- **Policy fields**: `id`, `name`, `enabled` (boolean)
- **Rules array** (`options.policies.*.rules`): Ordered rules within each policy
- **Rule fields**: `id`, `name`, `type` (unrestricted string), `weight` (integer for priority), `enabled` (boolean)
- **Rule options** (`options.policies.*.rules.*.options`): Open table for rule-specific configuration

## Example Configuration
```toml
[middleware.my_policies]
type = "policies"

[[middleware.my_policies.options.policies]]
id = "access_control"
name = "IP Access Control"
enabled = true

[[middleware.my_policies.options.policies.rules]]
id = "allow_internal"
name = "Allow Internal IPs"
type = "ip_allow"
weight = 100
enabled = true
[middleware.my_policies.options.policies.rules.options]
ip_addresses = ["10.0.0.0/8", "192.168.0.0/16"]

[[middleware.my_policies.options.policies.rules]]
id = "deny_blocklist"
name = "Block Known Bad IPs"
type = "ip_deny"
weight = 90
enabled = true
[middleware.my_policies.options.policies.rules.options]
ip_addresses = ["203.0.113.0/24"]
```

## Implementation Requirements

### File Structure
Create new file: `src/models/middleware/types/policies.rs`

Add to `src/models/middleware/types/mod.rs`:
```rust
pub mod policies;
```

### Data Structures
```rust
#[derive(Debug, Clone)]
pub struct PoliciesConfig {
    pub policies: Vec<Policy>,
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub id: Option<String>,
    pub name: Option<String>,
    pub enabled: bool,  // Default: true
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: Option<String>,
    pub name: Option<String>,
    pub rule_type: String,  // Maps to schema field "type"
    pub weight: i64,  // Default: 0, higher = higher priority
    pub enabled: bool,  // Default: true
    pub options: HashMap<String, serde_json::Value>,
}
```

### Core Middleware
```rust
pub struct PoliciesMiddleware {
    policies: Vec<Policy>,
}

impl PoliciesMiddleware {
    pub fn new(config: PoliciesConfig) -> Result<Self, String> {
        // Filter and sort policies/rules
        // - Only include enabled policies
        // - Only include enabled rules
        // - Sort rules by weight (descending)
        
        Ok(Self { policies })
    }
}
```

### Middleware Trait Implementation
Follow the pattern from `path_filter.rs`:
- Implement `async fn left()` for request-side processing
- Implement `async fn right()` for response-side (typically passthrough)
- Evaluate rules in weight order (highest weight first)
- Support "first match wins" or "all rules" evaluation strategy

### Rule Types to Support (Initial Phase)
1. **ip_allow**: Allow requests from specified IP addresses/ranges
   - Options: `ip_addresses: Vec<String>` (CIDR notation support)
2. **ip_deny**: Deny requests from specified IP addresses/ranges
   - Options: `ip_addresses: Vec<String>`
3. **rate_limit**: Basic rate limiting per IP or client
   - Options: `max_requests: u32`, `window_seconds: u32`

### Configuration Parser
```rust
pub fn parse_config(options: &HashMap<String, Value>) -> Result<PoliciesConfig, String> {
    // Extract and validate policies array
    // For each policy:
    //   - Parse id, name, enabled (default true)
    //   - Extract and validate rules array
    //   - For each rule:
    //     - Parse id, name, type, weight (default 0), enabled (default true)
    //     - Extract options as HashMap<String, Value>
    
    Ok(PoliciesConfig { policies })
}
```

### Registration
Add to `src/models/middleware/middleware.rs` in `create_builtin_middleware_type()`:
```rust
"policies" => {
    let config = crate::models::middleware::types::policies::parse_config(options)?;
    Ok(Box::new(PoliciesMiddleware::new(config)?))
}
```

## Evaluation Strategy

### Request Processing (left side)
**Core Logic**: A request is ACCEPTED only if at least ONE allow rule matches AND zero deny rules match.

1. Initialize tracking: `has_allow = false`, `has_deny = false`
2. Iterate through all enabled policies
3. For each policy, iterate through rules ordered by weight (descending)
4. For each enabled rule, evaluate based on rule type:
   - Extract relevant request metadata (IP, path, headers, etc.)
   - Apply rule-specific logic
   - If rule matches:
     - If rule type is ALLOW: set `has_allow = true`
     - If rule type is DENY: set `has_deny = true`
   - Continue evaluating ALL remaining rules (not first-match-wins)
5. After ALL rules evaluated:
   - If `has_deny = true`: DENY request (403, set `skip_backends`)
   - Else if `has_allow = false`: DENY request (implicit deny - no allow rule matched)
   - Else: ACCEPT request (has at least one allow, no denies)

**Note**: Weight determines evaluation ORDER but does NOT stop evaluation. All enabled rules are evaluated to check for any deny rules.

### Accessing Request Metadata
Use `envelope.request_details.metadata` to access:
- `"remote_addr"` or `"client_ip"` for IP-based rules
- `"path"` for path-based rules
- `"method"` for method-based rules
- Custom headers as needed

### Response on Policy Violation
Similar to path_filter:
```rust
envelope.request_details.metadata.insert("skip_backends".to_string(), "true".to_string());
envelope.normalized_data = Some(serde_json::json!({
    "response": {
        "status": 403,  // or 404
        "body": "Access denied by policy"
    }
}));
```

## Testing Requirements
Create `#[cfg(test)] mod tests` with:
1. Basic policy evaluation (allow/deny)
2. Weight-based priority ordering
3. Enabled/disabled policies and rules
4. IP matching (allow and deny)
5. Multiple policies interaction
6. Empty/invalid configuration handling
7. Missing fields with defaults

Follow test patterns from `path_filter.rs`.

## Reference Files
- **Schema**: `../harmony-dsl/harmony-pipeline-schema.toml` (lines 283-359)
- **Pattern reference**: `src/models/middleware/types/path_filter.rs`
- **Middleware trait**: `src/models/middleware/middleware.rs`
- **Instance config**: `src/models/middleware/instance.rs`

## Implementation Notes
1. Start with IP allow/deny rules only
2. Add rate limiting in a second phase
3. Consider adding metrics/logging for policy hits
4. Use `tracing::debug!` for evaluation traces
5. Use `tracing::warn!` for policy violations
6. Validate CIDR notation for IP addresses using `ipnetwork` crate
7. Consider caching parsed IP ranges for performance

## Future Extensions
- Path-based rules
- Header matching rules
- Time-based rules (time windows)
- Geolocation-based rules
- Custom rule types via dynamic loading
- Policy sets/inheritance
- Audit logging integration
