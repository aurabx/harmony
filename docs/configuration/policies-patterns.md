---
title: Policy Patterns and Troubleshooting
description: Common policy patterns, best practices, troubleshooting, and examples.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/policies-middleware.md
---

# Policy Patterns and Troubleshooting

Common policy patterns, best practices, troubleshooting, and examples. This page is part of [Policies Middleware](policies-middleware.md).

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
- The client IP must be in 192.168.0.0/16. The `ip_allow` rule is a required gate.
- The request must not target /admin.
- The geo rule only adds an allow. It does not restrict on its own if another rule allows the request.

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
