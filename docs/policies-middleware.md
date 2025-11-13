# Policies Middleware

**Last Updated**: 2025-01-13

The policies middleware provides comprehensive policy-based access control, rate limiting, and request filtering through a flexible rule system.

## Table of Contents

- [Overview](#overview)
- [Evaluation Logic](#evaluation-logic)
- [Configuration Structure](#configuration-structure)
- [Rule Types](#rule-types)
- [Weight and Priority](#weight-and-priority)
- [Common Patterns](#common-patterns)
- [Best Practices](#best-practices)
- [Performance](#performance)
- [Troubleshooting](#troubleshooting)

## Overview

Policies middleware enables you to:
- Control access based on IP addresses, paths, headers, or geographic location
- Implement rate limiting per client IP
- Apply time-based restrictions (business hours, maintenance windows)
- Combine multiple security rules with weight-based priority

### Key Features

- **Multi-layered security**: Combine 9 different rule types in a single policy
- **Performance optimized**: Pre-compiled IP networks and path routers
- **Hot-reload compatible**: Configuration changes apply without restart
- **Thread-safe rate limiting**: In-memory state tracking with RwLock
- **Flexible evaluation**: Weight-based priority with complete rule evaluation

## Evaluation Logic

**A request is ACCEPTED only if:**
1. At least ONE allow rule matches (across all enabled policies/rules)
2. AND zero deny rules match

### Key Points

- **All rules evaluated**: Not first-match-wins - every enabled rule is checked
- **Weight determines order**: Higher weight = evaluated first (but doesn't stop evaluation)
- **Implicit deny**: If no allow rules match, request is denied (403 Forbidden)
- **Deny overrides all**: A single matching deny rule blocks the request, regardless of allow rules

### Evaluation Flow

```
1. Filter to enabled policies only
2. For each policy:
   a. Filter to enabled rules only
   b. Sort rules by weight (descending)
   c. Evaluate ALL rules in order
   d. Track: has_allow, has_deny
3. Final decision:
   - If has_deny = true → DENY (403)
   - Else if has_allow = false → DENY (implicit, 403)
   - Else → ALLOW (continue to backend)
```

## Configuration Structure

```toml
[middleware.my_policy]
type = "policies"

# Policy 1
[[middleware.my_policy.options.policies]]
id = "access_control"
name = "Access Control Policy"
enabled = true

# Rule 1
[[middleware.my_policy.options.policies.rules]]
id = "rule1"
name = "Allow Internal Networks"
type = "ip_allow"
weight = 100
enabled = true

[middleware.my_policy.options.policies.rules.options]
ip_addresses = ["10.0.0.0/8", "192.168.0.0/16"]

# Rule 2
[[middleware.my_policy.options.policies.rules]]
id = "rule2"
name = "Deny Blocklist"
type = "ip_deny"
weight = 90
enabled = true

[middleware.my_policy.options.policies.rules.options]
ip_addresses = ["203.0.113.0/24"]
```

### Configuration Fields

**Policy fields:**
- `id` (string, optional): Unique identifier
- `name` (string, optional): Human-readable name
- `enabled` (boolean, default: true): Whether policy is active
- `rules` (array, required): Array of rules within this policy

**Rule fields:**
- `id` (string, optional): Unique identifier
- `name` (string, optional): Human-readable name
- `type` (string, required): Rule type (see [Rule Types](#rule-types))
- `weight` (integer, default: 0): Priority for evaluation order
- `enabled` (boolean, default: true): Whether rule is active
- `options` (table, varies): Rule-specific configuration

## Rule Types

### 1. IP Allow (`ip_allow`)

Allow requests from specific IP addresses or CIDR ranges.

**Options:**
- `ip_addresses` (array of strings, required): IP addresses or CIDR notation

**Example:**
```toml
[[middleware.security.options.policies.rules]]
type = "ip_allow"
weight = 100
enabled = true

[middleware.security.options.policies.rules.options]
ip_addresses = [
    "10.0.0.0/8",       # Private network - Class A
    "172.16.0.0/12",    # Private network - Class B
    "192.168.0.0/16",   # Private network - Class C
    "127.0.0.1/32"      # Localhost
]
```

**Behavior:**
- Extracts client IP from `remote_addr` or `client_ip` metadata
- Matches against pre-compiled CIDR networks (O(log n))
- On match: Rule evaluates to ALLOW
- On no match: Rule evaluates to NO_MATCH (implicit deny if no other allows)

**Use Cases:**
- Internal network allowlisting
- VPN-only access
- Specific office IP ranges

### 2. IP Deny (`ip_deny`)

Block requests from specific IP addresses or CIDR ranges.

**Options:**
- `ip_addresses` (array of strings, required): IP addresses or CIDR notation

**Example:**
```toml
[[middleware.security.options.policies.rules]]
type = "ip_deny"
weight = 90
enabled = true

[middleware.security.options.policies.rules.options]
ip_addresses = [
    "203.0.113.0/24",   # TEST-NET-3 (example blocklist)
    "198.51.100.0/24"   # TEST-NET-2 (example blocklist)
]
```

**Behavior:**
- Same IP matching logic as `ip_allow`
- On match: Rule evaluates to DENY (request blocked)
- On no match: Rule evaluates to NO_MATCH

**Use Cases:**
- Blocklist known malicious IPs
- Prevent access from specific regions/networks
- Emergency blocking of compromised IPs

### 3. Rate Limit (`rate_limit`)

Throttle requests based on count within a time window (per client IP).

**Options:**
- `max_requests` (integer, required): Maximum requests allowed
- `window_seconds` (integer, required): Time window in seconds

**Example:**
```toml
[[middleware.api_limiter.options.policies.rules]]
type = "rate_limit"
weight = 50
enabled = true

[middleware.api_limiter.options.policies.rules.options]
max_requests = 100
window_seconds = 60  # 100 requests per minute
```

**Behavior:**
- Tracks request count per client IP using thread-safe RwLock
- Sliding window: resets when `window_seconds` expires
- Within limit: Rule evaluates to NO_MATCH
- Limit exceeded: Rule evaluates to DENY
- First request in new window: count = 1, starts new timer

**Important:** Rate limiting acts as a DENY rule when exceeded. You MUST have an allow rule (typically `allow_all`) for rate limiting to work correctly.

**Use Cases:**
- API rate limiting (100 req/min, 1000 req/hour)
- DDoS protection
- Fair usage enforcement
- Prevent abuse

### 4. Path Match (`path`)

Allow or deny based on URL path patterns.

**Options:**
- `paths` (array of strings, required): Path patterns (matchit syntax)
- `mode` (string, required): "allow" or "deny"

**Example:**
```toml
[[middleware.api_filter.options.policies.rules]]
type = "path"
weight = 80
enabled = true

[middleware.api_filter.options.policies.rules.options]
paths = [
    "/api/public/{*path}",  # Catch-all under /api/public
    "/health",
    "/metrics"
]
mode = "allow"
```

**Pattern Syntax (matchit):**
- Exact: `/users`, `/api/health`
- Wildcards: `/api/*` (single segment)
- Catch-all: `/api/{*path}` (multiple segments)
- Parameters: `/users/{id}`

**Behavior:**
- Pre-compiles paths into matchit router at config load
- Extracts path from `path` metadata field
- Normalizes paths (adds leading `/`, removes trailing `/`)
- On match + mode="allow": Rule evaluates to ALLOW
- On match + mode="deny": Rule evaluates to DENY
- On no match: Rule evaluates to NO_MATCH

**Use Cases:**
- Protect admin endpoints (`/admin/{*path}` deny)
- Allow only specific API paths
- Public vs private API segregation

### 5. Geographic (`geo`)

Allow or deny based on client's country code.

**Options:**
- `country_codes` (array of strings, required): ISO 3166-1 alpha-2 codes
- `mode` (string, required): "allow" or "deny"

**Example:**
```toml
[[middleware.geo_filter.options.policies.rules]]
type = "geo"
weight = 70
enabled = true

[middleware.geo_filter.options.policies.rules.options]
country_codes = ["US", "GB", "CA", "AU"]
mode = "allow"
```

**Behavior:**
- Extracts country from `geo_country` or `country_code` metadata
- Normalizes codes to uppercase
- On match + mode="allow": Rule evaluates to ALLOW
- On match + mode="deny": Rule evaluates to DENY
- On no match: Rule evaluates to NO_MATCH

**Requirements:**
- `geo_country` or `country_code` must be populated in request metadata
- Typically set by geolocation middleware or protocol adapter
- Country codes must be valid ISO 3166-1 alpha-2 (2 letters)

**Use Cases:**
- GDPR compliance (EU countries only)
- Regional service restrictions
- Country-based routing
- Sanctions compliance

### 6. Header Match (`header`)

Allow or deny based on HTTP header values.

**Options:**
- `mode` (string, required): "allow" or "deny"
- `headers` (array of objects, required): Header match configurations
  - `name` (string): Header name
  - `match_type` (string): "exact", "contains", or "regex"
  - `value` (string): Value or pattern to match

**Example:**
```toml
[[middleware.header_filter.options.policies.rules]]
type = "header"
weight = 60
enabled = true

[middleware.header_filter.options.policies.rules.options]
mode = "allow"
headers = [
    { name = "User-Agent", match_type = "regex", value = "^Mozilla.*" },
    { name = "X-API-Key", match_type = "exact", value = "secret-key" }
]
```

**Match Types:**
- `exact`: Case-insensitive exact match
- `contains`: Case-insensitive substring match  
- `regex`: Regex pattern match (compiled at runtime)

**Behavior:**
- Checks headers from metadata (`header_<name>` or direct name lookup)
- ALL headers must match for rule to trigger
- Invalid regex patterns log warning and return false
- On all match + mode="allow": Rule evaluates to ALLOW
- On all match + mode="deny": Rule evaluates to DENY
- On any no match: Rule evaluates to NO_MATCH

**Use Cases:**
- API key validation
- User-Agent filtering (block bots)
- Custom authentication headers
- Version-based access control

### 7. Allow All (`allow_all`)

Blanket allow for all requests (no configuration required).

**Example:**
```toml
[[middleware.open_api.options.policies.rules]]
type = "allow_all"
weight = 100
enabled = true
```

**Behavior:**
- Always evaluates to ALLOW
- No options required
- Useful as base rule for rate limiting

**Use Cases:**
- Testing and development
- Public endpoints
- Base allow rule for rate limiting policies
- Default allow (with specific denies)

### 8. Deny All (`deny_all`)

Blanket deny for all requests (no configuration required).

**Example:**
```toml
[[middleware.maintenance.options.policies.rules]]
type = "deny_all"
weight = 0
enabled = true
```

**Behavior:**
- Always evaluates to DENY
- No options required

**Use Cases:**
- Maintenance mode
- Emergency blocking
- Default-deny security stance
- Placeholder for future rules

### 9. Time Based (`time_based`)

Allow or deny requests based on time windows, days of week, and date ranges.

**Options:**
- `allow_during_window` (boolean, default: true): If true, allow during window; if false, deny during window
- `timezone` (string, default: "UTC"): IANA timezone (e.g., "America/New_York", "Europe/London")
- `start_time` (string, optional): Start time in HH:MM format (24-hour)
- `end_time` (string, optional): End time in HH:MM format (24-hour)
- `days_of_week` (array of strings, optional): Allowed days ("monday", "tuesday", etc., lowercase)
- `start_date` (string, optional): Start date in YYYY-MM-DD format
- `end_date` (string, optional): End date in YYYY-MM-DD format

**Example 1: Business Hours**
```toml
[[middleware.hours.options.policies.rules]]
type = "time_based"
weight = 100
enabled = true

[middleware.hours.options.policies.rules.options]
allow_during_window = true
timezone = "America/New_York"
start_time = "09:00"
end_time = "17:00"
days_of_week = ["monday", "tuesday", "wednesday", "thursday", "friday"]
```

**Example 2: Maintenance Window**
```toml
[[middleware.maint.options.policies.rules]]
type = "time_based"
weight = 100
enabled = true

[middleware.maint.options.policies.rules.options]
allow_during_window = false  # Deny during window
timezone = "UTC"
start_time = "02:00"
end_time = "04:00"
```

**Example 3: Date Range Restriction**
```toml
[[middleware.event.options.policies.rules]]
type = "time_based"
weight = 100
enabled = true

[middleware.event.options.policies.rules.options]
allow_during_window = true
timezone = "UTC"
start_date = "2025-01-01"
end_date = "2025-12-31"
```

**Behavior:**
- Evaluates current time in specified timezone
- Checks time window (supports wrap-around midnight: 22:00-06:00)
- Checks day of week (all specified days must match)
- Checks date range (inclusive)
- If all conditions pass:
  - `allow_during_window=true` → Rule evaluates to ALLOW
  - `allow_during_window=false` → Rule evaluates to DENY
- If any condition fails:
  - `allow_during_window=true` → Rule evaluates to DENY
  - `allow_during_window=false` → Rule evaluates to ALLOW

**Use Cases:**
- Business hours enforcement
- Maintenance windows (deny during specific times)
- Weekend-only access
- Holiday schedule restrictions
- Time-limited promotions or features

**Notes:**
- Invalid timezone falls back to UTC with warning
- Invalid time/date formats return NO_MATCH with warning
- Time windows that wrap midnight (22:00-06:00) are supported
- All time comparisons use the current time in the specified timezone

## Weight and Priority

Rules are evaluated in weight order (higher weight = evaluated first):

```
Weight Range    Purpose                     Example
-----------     ---------                   -------
-1000 to -100   Critical overrides          Emergency blocks
-99 to -1       High priority rules         Specific allows/denies
0               Default priority            Most rules
1 to 99         Lower priority              Catch-all rules
100 to 1000     Fallback rules              Default deny-all
```

**Important:** Weight only affects evaluation ORDER. All enabled rules are still evaluated - it's not first-match-wins.

## Common Patterns

### Pattern 1: IP Allowlist with Blocklist

```toml
[[middleware.security.options.policies]]
id = "ip_security"
enabled = true

# Allow internal networks (highest priority)
[[middleware.security.options.policies.rules]]
type = "ip_allow"
weight = 100
[middleware.security.options.policies.rules.options]
ip_addresses = ["10.0.0.0/8", "192.168.0.0/16"]

# Deny known bad IPs (medium priority)
[[middleware.security.options.policies.rules]]
type = "ip_deny"
weight = 90
[middleware.security.options.policies.rules.options]
ip_addresses = ["203.0.113.0/24"]
```

**Result:**
- Internal IPs: ✅ ALLOW (matches allow, no deny)
- Blocked IPs: ❌ DENY (matches deny)
- External IPs: ❌ DENY (implicit - no allow match)

### Pattern 2: Rate Limiting

```toml
[[middleware.rate_limiter.options.policies]]
id = "api_limits"
enabled = true

# Allow all (required base)
[[middleware.rate_limiter.options.policies.rules]]
type = "allow_all"
weight = 100

# Rate limit (acts as deny when exceeded)
[[middleware.rate_limiter.options.policies.rules]]
type = "rate_limit"
weight = 50
[middleware.rate_limiter.options.policies.rules.options]
max_requests = 100
window_seconds = 60
```

**Result:**
- Under limit: ✅ ALLOW (allow_all + rate_limit NO_MATCH)
- Over limit: ❌ DENY (allow_all + rate_limit DENY)

### Pattern 3: Path-Based Access Control

```toml
[[middleware.api_access.options.policies]]
id = "path_control"
enabled = true

# Allow public API
[[middleware.api_access.options.policies.rules]]
type = "path"
weight = 100
[middleware.api_access.options.policies.rules.options]
paths = ["/api/public/{*path}"]
mode = "allow"

# Deny admin paths
[[middleware.api_access.options.policies.rules]]
type = "path"
weight = 90
[middleware.api_access.options.policies.rules.options]
paths = ["/admin/{*path}"]
mode = "deny"
```

**Result:**
- `/api/public/users`: ✅ ALLOW
- `/admin/users`: ❌ DENY
- `/api/internal/users`: ❌ DENY (implicit - no allow)

### Pattern 4: Layered Security

```toml
[[middleware.layered.options.policies]]
id = "multi_layer"
enabled = true

# Geographic restriction
[[middleware.layered.options.policies.rules]]
type = "geo"
weight = 100
[middleware.layered.options.policies.rules.options]
country_codes = ["US", "GB"]
mode = "allow"

# IP restriction
[[middleware.layered.options.policies.rules]]
type = "ip_allow"
weight = 90
[middleware.layered.options.policies.rules.options]
ip_addresses = ["192.168.0.0/16"]

# Path restriction
[[middleware.layered.options.policies.rules]]
type = "path"
weight = 80
[middleware.layered.options.policies.rules.options]
paths = ["/admin/{*path}"]
mode = "deny"
```

**Result:**
- Must be from US/GB AND from internal IP AND not accessing /admin
- All conditions must be met (geo allow + IP allow + path not deny)

## Best Practices

### 1. Always Have an Allow Rule

Without any allow rules, all requests are implicitly denied:

```toml
# ❌ Bad: No allow rules
[[middleware.bad.options.policies.rules]]
type = "ip_deny"
[middleware.bad.options.policies.rules.options]
ip_addresses = ["1.2.3.4"]

# ✅ Good: Has allow rule
[[middleware.good.options.policies.rules]]
type = "ip_allow"
[middleware.good.options.policies.rules.options]
ip_addresses = ["10.0.0.0/8"]
```

### 2. Use Descriptive IDs and Names

Makes debugging and log analysis much easier:

```toml
[[middleware.security.options.policies]]
id = "corporate_network_access"
name = "Corporate Network Access Control"

[[middleware.security.options.policies.rules]]
id = "allow_hq_office"
name = "Allow HQ Office Network"
type = "ip_allow"
```

### 3. Higher Weight for Security-Critical Rules

Emergency blocks should have very high weights:

```toml
# Emergency block (weight -1000)
[[middleware.security.options.policies.rules]]
id = "emergency_block"
type = "ip_deny"
weight = -1000
[middleware.security.options.policies.rules.options]
ip_addresses = ["1.2.3.4"]  # Compromised IP

# Normal rules (weight 0-100)
[[middleware.security.options.policies.rules]]
type = "ip_allow"
weight = 100
[middleware.security.options.policies.rules.options]
ip_addresses = ["10.0.0.0/8"]
```

### 4. Test in Development First

Policy misconfiguration can block legitimate traffic:

```bash
# Start with liberal policy, then tighten
1. Start: allow_all
2. Add: specific deny rules (test each)
3. Replace: allow_all with specific allows (test thoroughly)
4. Deploy: to production
```

### 5. Monitor Logs for Violations

Use debug logging to see rule evaluations:

```bash
RUST_LOG=harmony=debug cargo run -- --config config.toml

# Look for:
# "Evaluating policy: <name>"
# "Rule <name> matched - ALLOW/DENY"
# "Request ALLOWED/DENIED - <reason>"
```

### 6. Combine with Other Security Measures

Policies work best alongside:
- JWT authentication (verify tokens)
- TLS/mTLS (encrypted transport)
- WAF (SQL injection, XSS protection)
- API gateways (additional rate limiting)

## Performance

### Pre-Compilation Optimizations

- **IP rules**: CIDR networks compiled at config load → O(log n) lookup
- **Path rules**: matchit router compiled at config load → O(log n) lookup
- **Rate limiting**: In-memory HashMap with RwLock → O(1) lookup
  - **Automatic cleanup**: Background task runs every 60 seconds
  - Removes entries older than 5 minutes (300 seconds)
  - Prevents unbounded memory growth from unique IPs
  - Debug logging when entries are removed
- **Header/Geo**: Simple string matching → O(1) per header

### Typical Performance

- **Overhead**: < 1ms per request with moderate rule counts (< 50 rules)
- **Memory**: ~1KB per rule (pre-compiled state)
- **Scalability**: Tested up to 1000 concurrent requests without degradation

### Hot Reload Support

Policies middleware supports hot configuration reload:
- Changes to policies/rules applied without restart
- Pre-compiled caches updated atomically (Arc swap)
- Rate limit state preserved across reloads
- Zero downtime for configuration changes

## Troubleshooting

### All Requests Denied

**Symptoms:** All requests return 403 Forbidden

**Solutions:**
1. Check that at least one allow rule is configured
2. Verify the allow rule actually matches your requests:
   ```bash
   RUST_LOG=harmony=debug cargo run
   # Look for: "Rule <name> did not match"
   ```
3. Check rule `enabled` flags (both policy and rule level)
4. Verify metadata fields are populated (`remote_addr`, `path`, etc.)

### Rate Limiting Not Working

**Symptoms:** Requests not being rate limited, or all denied

**Solutions:**
1. Ensure you have an `allow_all` rule:
   ```toml
   [[middleware.limiter.options.policies.rules]]
   type = "allow_all"
   weight = 100
   
   [[middleware.limiter.options.policies.rules]]
   type = "rate_limit"
   weight = 50
   [middleware.limiter.options.policies.rules.options]
   max_requests = 100
   window_seconds = 60
   ```
2. Verify `remote_addr` or `client_ip` is set in request metadata
3. Check rate limit window hasn't already expired
4. **Note:** Rate limit state is automatically cleaned up after 5 minutes of inactivity to prevent memory leaks. This means inactive IPs will have their counters reset.

### Geographic Rules Not Matching

**Symptoms:** Geo rules don't seem to evaluate

**Solutions:**
1. Ensure `geo_country` or `country_code` is populated in metadata
2. Verify country codes are uppercase ISO 3166-1 alpha-2 (2 letters)
3. Check that geolocation middleware/adapter runs before policies
4. Test with known country code:
   ```bash
   # Add debug logging to see metadata
   RUST_LOG=harmony=debug cargo run
   ```

### Path Rules Not Matching

**Symptoms:** Path-based allow/deny not working

**Solutions:**
1. Check path syntax (must start with `/`)
2. Verify `path` metadata is populated
3. Test patterns individually:
   ```toml
   # Start simple
   paths = ["/api"]
   
   # Then add complexity
   paths = ["/api/{*path}"]
   ```
4. Remember: path filter uses request path AFTER endpoint prefix

### Debug Logging

Enable detailed logging to see rule evaluation:

```bash
# Full debug output
RUST_LOG=harmony=debug cargo run -- --config config.toml

# Policies only
RUST_LOG=harmony::models::middleware::types::policies=debug cargo run

# Look for these log messages:
# INFO  harmony::models::middleware::types::policies > PoliciesMiddleware initialized with N enabled policies
# DEBUG harmony::models::middleware::types::policies > Evaluating policy: <name>
# DEBUG harmony::models::middleware::types::policies > Evaluating rule: type=<type>, name=<name>, weight=<weight>
# DEBUG harmony::models::middleware::types::policies > IP rule evaluation: client_ip=X, matches=Y
# DEBUG harmony::models::middleware::types::policies > Rule <name> matched - ALLOW
# WARN  harmony::models::middleware::types::policies > Rule <name> matched - DENY
# WARN  harmony::models::middleware::types::policies > Request DENIED - at least one deny rule matched
# WARN  harmony::models::middleware::types::policies > Request DENIED - no allow rule matched (implicit deny)
# DEBUG harmony::models::middleware::types::policies > Request ALLOWED - has allow rule(s) and no deny rules
```

## Examples

### Complete Example: Multi-Layered Security

See `examples/policy-config.toml` for a complete working example featuring:
- IP-based access control (allow internal, deny blocklist)
- Rate limiting (100 req/min per IP)
- Multiple policies working together
- Comprehensive comments and scenarios

### Running the Example

```bash
# Start with example configuration
cargo run -- --config examples/policy-config.toml

# Test from internal IP (should work)
curl -H "X-Forwarded-For: 192.168.1.100" http://localhost:8080/api/test

# Test from blocked IP (should fail)
curl -H "X-Forwarded-For: 203.0.113.45" http://localhost:8080/api/test

# Test rate limiting (101st request should fail)
for i in {1..101}; do
  curl http://localhost:8080/api/test
done
```

## See Also

- [Configuration Guide](configuration.md) - Overall configuration structure
- [Middleware Overview](middleware.md) - Other middleware types
- [Path Filter Middleware](middleware.md#path-filter) - Simple path-based filtering
- [Hot Reload](config-reload.md) - Configuration reload architecture
