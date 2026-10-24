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

Each pattern shows the middleware, its policy, and its rules. Add the middleware name to a pipeline's `middleware` list to use it.

## Common Patterns

### Pattern 1: IP Allowlist with Blocklist

```toml
[middleware.security]
type = "policies"
[middleware.security.options]
policies = ["ip_security"]

[policies.ip_security]
id = "ip_security"
rules = ["allow_internal", "deny_bad_ips"]

[rules.allow_internal]
id = "allow_internal"
type = "ip_allow"
weight = 100
[rules.allow_internal.options]
ip_addresses = ["10.0.0.0/8", "192.168.0.0/16"]

[rules.deny_bad_ips]
id = "deny_bad_ips"
type = "ip_deny"
weight = 90
[rules.deny_bad_ips.options]
ip_addresses = ["10.66.0.0/16"]
```

**Result:**
- 10.1.2.3: ALLOW (in the allowlist, not in the blocklist)
- 10.66.1.1: DENY 403 (in the blocklist)
- 203.0.113.9: DENY 403 (outside the allowlist gate)

### Pattern 2: Rate Limiting

```toml
[middleware.rate_limiter]
type = "policies"
[middleware.rate_limiter.options]
policies = ["api_limits"]

[policies.api_limits]
id = "api_limits"
rules = ["allow_everyone", "per_ip_limit"]

[rules.allow_everyone]
id = "allow_everyone"
type = "allow_all"
weight = 100

[rules.per_ip_limit]
id = "per_ip_limit"
name = "Per-IP limit"
type = "rate_limit"
weight = 50
[rules.per_ip_limit.options]
max_requests = 100
window_seconds = 60
```

**Result:**
- Under the limit: ALLOW (`allow_all` allows, `rate_limit` returns no match)
- Over the limit: DENY 429

### Pattern 3: Path-Based Access Control

```toml
[middleware.api_access]
type = "policies"
[middleware.api_access.options]
policies = ["path_control"]

[policies.path_control]
id = "path_control"
rules = ["public_api", "block_admin"]

[rules.public_api]
id = "public_api"
type = "path"
weight = 100
[rules.public_api.options]
mode = "allow"
paths = ["/api/public/{*path}"]

[rules.block_admin]
id = "block_admin"
type = "path"
weight = 90
[rules.block_admin.options]
mode = "deny"
paths = ["/admin/{*path}"]
```

**Result:**
- `/api/public/users`: ALLOW
- `/admin/users`: DENY 403
- `/api/internal/users`: DENY 403 (implicit deny, no rule allows it)

### Pattern 4: Layered Security

```toml
[middleware.layered]
type = "policies"
[middleware.layered.options]
policies = ["multi_layer"]

[policies.multi_layer]
id = "multi_layer"
name = "Multi-Layer Security Policy"
rules = ["office_network", "block_admin", "block_bots", "per_ip_limit"]

[rules.office_network]
id = "office_network"
type = "ip_allow"
weight = 90
[rules.office_network.options]
ip_addresses = ["10.0.0.0/8", "192.168.0.0/16"]

[rules.block_admin]
id = "block_admin"
type = "path"
weight = 85
[rules.block_admin.options]
mode = "deny"
paths = ["/admin/{*path}"]

[rules.block_bots]
id = "block_bots"
type = "user_agent"
weight = 80
[rules.block_bots.options]
mode = "deny"
patterns = [{ label = "Bots", pattern = "/bot|crawler|spider/i" }]

[rules.per_ip_limit]
id = "per_ip_limit"
type = "rate_limit"
weight = 50
[rules.per_ip_limit.options]
max_requests = 100
window_seconds = 60
```

**Result:**
- The client IP must be in 10.0.0.0/8 or 192.168.0.0/16. The `ip_allow` rule is a required gate, and it supplies the ALLOW.
- Requests to `/admin/...` get 403.
- Clients whose `User-Agent` matches the bot pattern get 403.
- A client over 100 requests a minute gets 429.

### Pattern 5: Read-Only API

```toml
[middleware.readonly_api]
type = "policies"
[middleware.readonly_api.options]
policies = ["readonly"]

[policies.readonly]
id = "readonly"
rules = ["read_methods", "block_bots"]

[rules.read_methods]
id = "read_methods"
type = "method"
weight = 100
[rules.read_methods.options]
mode = "allow"
methods = ["GET", "HEAD"]

[rules.block_bots]
id = "block_bots"
type = "user_agent"
weight = 90
[rules.block_bots.options]
mode = "deny"
patterns = [{ pattern = "/bot|crawler|spider/i" }]
```

**Result:**
- `GET` and `HEAD`: ALLOW
- Any other method: DENY 405
- A bot `User-Agent`: DENY 403

### Pattern 6: Business Hours for Internal Clients

```toml
[middleware.business_hours]
type = "policies"
[middleware.business_hours.options]
policies = ["hours"]

[policies.hours]
id = "hours"
rules = ["office_hours", "internal_only"]

[rules.office_hours]
id = "office_hours"
type = "time_based"
weight = 100
[rules.office_hours.options]
allow_during_window = true
timezone = "America/New_York"
start_time = "09:00"
end_time = "17:00"
days_of_week = ["mon", "tue", "wed", "thu", "fri"]

[rules.internal_only]
id = "internal_only"
type = "ip_allow"
weight = 90
[rules.internal_only.options]
ip_addresses = ["10.0.0.0/8"]
```

**Result:**
- An internal client during business hours: ALLOW
- An internal client outside business hours: DENY 403. The `time_based` rule returns DENY outside its window, and a DENY wins over the `ip_allow` match.
- An external client at any time: DENY 403 (outside the allowlist gate)

One middleware cannot give internal clients access at all hours while it limits other clients to business hours.

## Best Practices

### 1. Always Have an Allow Rule

If no rule returns ALLOW, every request is denied. A policy with only `ip_deny` or `rate_limit` rules denies everything. Add `allow_all` or another allow rule.

### 2. Use Descriptive IDs and Names

Deny logs and error messages show the rule's `name`, or `unnamed` if it has none. The allowlist gate logs show rule ids.

```toml
[policies.corporate_network_access]
id = "corporate_network_access"
name = "Corporate Network Access Control"
rules = ["allow_hq_office"]

[rules.allow_hq_office]
id = "allow_hq_office"
name = "Allow HQ Office Network"
type = "ip_allow"
[rules.allow_hq_office.options]
ip_addresses = ["192.168.10.0/24"]
```

### 3. Do Not Rely on Weight to Block

Any DENY blocks the request, whatever its weight. An emergency `ip_deny` rule works at any weight. Weight only decides which deny is reported when several rules deny. The lowest-weight DENY is reported. See [Weight and Priority](policies-middleware.md#weight-and-priority).

### 4. Test in Development First

A wrong policy can block legitimate traffic. Tighten in steps:

1. Start with `allow_all`.
2. Add specific deny rules and test each one.
3. Replace `allow_all` with specific allow rules and test again.
4. Deploy to production.

### 5. Watch the Logs

Run with debug logging to see each rule's result. See [Debug Logging](#debug-logging).

### 6. Combine with Other Security Measures

Use policies alongside JWT authentication, TLS, and any upstream WAF or gateway.

## Troubleshooting

### All Requests Denied

**Symptoms:** Every request returns 403 Forbidden.

**Solutions:**
1. Check that at least one rule can return ALLOW.
2. Check for an allow-mode `query_parameter` rule. Unless an earlier middleware sets query metadata, it denies every HTTP request. See [Known Issues](policies-middleware.md#known-issues).
3. Check `days_of_week` uses `mon`, `tue` and so on. Full day names deny every request in allow mode.
4. Check the client IP matches at least one `ip_allow` rule. The allowlists form a union, so one match is enough. Any enabled `ip_allow` rule turns on the gate.
5. Check the `enabled` flags on policies and rules.
6. Run with debug logging and look for `did not match` and `matched - DENY` lines.

### Rate Limiting Not Working

**Symptoms:** Requests are never limited, or every request is denied.

**Solutions:**
1. Make sure another rule returns ALLOW, such as `allow_all`.
2. Check `remote_addr` is set. Without it, all clients share one `unknown` count.
3. Keep `window_seconds` at 300 or less. Longer windows reset early.
4. Inactive clients lose their count after about five minutes.

### Header, Query Parameter, or Geo Rules Not Matching

These rules read metadata keys that nothing in Harmony sets by default. Without an earlier middleware that sets those keys, they do not match HTTP requests. See [Known Issues](policies-middleware.md#known-issues).

### Path Rules Not Matching

**Solutions:**
1. Each pattern must start with `/`.
2. The rule sees the path after the endpoint's `path_prefix`. With `path_prefix = "/api"`, a request to `/api/users` is checked as `/users`.
3. Use `{*name}` for a catch-all. A bare `*` is a literal character.
4. A trailing `/` is not removed. `/health` does not match `/health/`.

### Debug Logging

```bash
# Full debug output
RUST_LOG=harmony=debug cargo run -- --config config.toml

# Policies only
RUST_LOG=harmony::models::middleware::types::policies=debug cargo run -- --config config.toml
```

Log lines to look for:

```
INFO  PoliciesMiddleware initialized with N enabled policies (instance_key=<name>)
DEBUG Evaluating policy: <name>
DEBUG Evaluating rule: type=<type>, name=Some("<name>"), weight=<weight>
DEBUG IP rule evaluation: client_ip=<ip>, matches=<bool>, is_allow=<bool>
DEBUG Rule "<name>" matched - ALLOW
WARN  Rule "<name>" (type: <type>) matched - DENY
DEBUG Rule "<name>" did not match
WARN  Request DENIED by rule '<name>' (type: <type>)
WARN  Request DENIED - no allow rule matched (implicit deny)
INFO  Request DENIED - client_ip=<ip> matches none of the IP allowlist rules (rules: <ids>)
DEBUG Request ALLOWED - has allow rule(s) and no deny rules
```

### Testing a Policy by Hand

The policies middleware reads the TCP peer address. It never reads `X-Forwarded-For`, so you cannot fake a client IP with that header. Test IP rules from a real host in the range, or add `127.0.0.1/32` to a test allowlist.

```bash
# Rate limit: with max_requests = 100, the 101st request gets 429
for i in $(seq 1 101); do
  curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8080/api/test
done
```
