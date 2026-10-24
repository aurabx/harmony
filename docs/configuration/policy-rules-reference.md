---
title: Policy Rules Quick Reference
description: Quick reference for every policy rule type and its options.
status: active
updated: 2026-10-24
type: Configuration
---

# Policy Rules Quick Reference

This document provides a quick reference for all available policy rule types in the Harmony Proxy policies middleware.

## Table of Contents

- [Overview](#overview)
- [Rule Types Summary](#rule-types-summary)
- [IP-Based Rules](#ip-based-rules)
- [Path-Based Rules](#path-based-rules)
- [Header-Based Rules](#header-based-rules)
- [Geographic Rules](#geographic-rules)
- [Rate Limiting Rules](#rate-limiting-rules)
- [Time-Based Rules](policy-rules-reference-request.md#time-based-rules)
- [HTTP Method Rules](policy-rules-reference-request.md#http-method-rules)
- [User Agent Rules](policy-rules-reference-request.md#user-agent-rules)
- [Content Type Rules](policy-rules-reference-request.md#content-type-rules)
- [Query Parameter Rules](policy-rules-reference-request.md#query-parameter-rules)
- [Control Rules](#control-rules)

## Overview

All policy rules follow this structure:

```toml
[[middleware.my_policy.options.policies.rules]]
id = "rule_identifier"              # Optional: Unique ID
name = "Human Readable Name"        # Optional: Descriptive name
type = "rule_type"                  # Required: Rule type (see below)
weight = 100                        # Optional: Priority (default: 0)
enabled = true                      # Optional: Enable/disable (default: true)

[middleware.my_policy.options.policies.rules.options]
# Rule-specific options go here
```

## Rule Types Summary

| Type | Purpose | Mode | Options |
|------|---------|------|---------|
| `ip_allow` | Allow specific IPs | N/A | `ip_addresses` |
| `ip_deny` | Block specific IPs | N/A | `ip_addresses` |
| `path` | Filter by URL path | allow/deny | `paths`, `mode` |
| `header` | Match header values | allow/deny | `headers`, `mode` |
| `geo` | Filter by country | allow/deny | `country_codes`, `mode` |
| `rate_limit` | Throttle requests | N/A | `max_requests`, `window_seconds` |
| `time_based` | Time restrictions | allow/deny | `timezone`, `start_time`, `end_time`, `days_of_week`, `start_date`, `end_date`, `allow_during_window` |
| `method` | Filter by HTTP method | allow/deny | `methods`, `mode` |
| `user_agent` | Match User-Agent | allow/deny | `patterns`, `mode` |
| `content_type` | Filter Content-Type | allow/deny | `content_types`, `mode` |
| `query_parameter` | Match query params | allow/deny | `parameters`, `mode` |
| `allow_all` | Allow everything | N/A | None |
| `deny_all` | Block everything | N/A | None |

## IP-Based Rules

### IP Allow

```toml
[[middleware.security.options.policies.rules]]
type = "ip_allow"
weight = 100
enabled = true

[middleware.security.options.policies.rules.options]
ip_addresses = [
    "10.0.0.0/8",         # Private network
    "192.168.1.0/24",     # Office network
    "203.0.113.45"        # Single IP
]
```

An `ip_allow` rule is a required gate for the middleware. A client IP must match at least one `ip_allow` rule, in any policy. Otherwise it gets 403, even if another rule allows the request. `ip_allow` rules form a union, within a policy and across policies. "Office OR VPN" can be two policies. An unknown client IP also gets 403. See [IP Allowlist Gate](policies-middleware.md#ip-allowlist-gate).

### IP Deny

```toml
[[middleware.security.options.policies.rules]]
type = "ip_deny"
weight = 90
enabled = true

[middleware.security.options.policies.rules.options]
ip_addresses = [
    "203.0.113.0/24",     # Blocked range
    "198.51.100.45"       # Blocked IP
]
```

## Path-Based Rules

### Allow Specific Paths

```toml
[[middleware.api_filter.options.policies.rules]]
type = "path"
weight = 80
enabled = true

[middleware.api_filter.options.policies.rules.options]
mode = "allow"
paths = [
    "/api/public/{*path}",    # Catch-all
    "/health",                # Exact match
    "/metrics"
]
```

### Deny Admin Paths

```toml
[[middleware.api_filter.options.policies.rules]]
type = "path"
weight = 85
enabled = true

[middleware.api_filter.options.policies.rules.options]
mode = "deny"
paths = [
    "/admin/{*path}",
    "/internal/{*path}"
]
```

## Header-Based Rules

### Header Match

```toml
[[middleware.header_filter.options.policies.rules]]
type = "header"
weight = 60
enabled = true

[middleware.header_filter.options.policies.rules.options]
mode = "allow"
headers = [
    { name = "X-API-Key", match_type = "exact", value = "secret-key" },
    { name = "User-Agent", match_type = "regex", value = "^Mozilla.*" },
    { name = "X-Custom", match_type = "contains", value = "value" }
]
```

## Geographic Rules

### Allow Specific Countries

```toml
[[middleware.geo_filter.options.policies.rules]]
type = "geo"
weight = 70
enabled = true

[middleware.geo_filter.options.policies.rules.options]
mode = "allow"
country_codes = ["US", "GB", "CA", "AU"]
```

### Block Countries

```toml
[[middleware.geo_filter.options.policies.rules]]
type = "geo"
weight = 70
enabled = true

[middleware.geo_filter.options.policies.rules.options]
mode = "deny"
country_codes = ["XX", "YY"]  # ISO 3166-1 alpha-2
```

## Rate Limiting Rules

### Basic Rate Limit

```toml
# Must be used with allow_all rule
[[middleware.rate_limiter.options.policies.rules]]
type = "allow_all"
weight = 100

[[middleware.rate_limiter.options.policies.rules]]
type = "rate_limit"
weight = 50
enabled = true

[middleware.rate_limiter.options.policies.rules.options]
max_requests = 100        # Maximum requests
window_seconds = 60       # Per 60 seconds (1 minute)
```

## Policy Rules Reference: Request Attributes

Quick reference for the time, method, user agent, content type, and query parameter rules. See [Policy Rules Reference: Request Attributes](policy-rules-reference-request.md).

## Control Rules

### Allow All

```toml
[[middleware.open_api.options.policies.rules]]
type = "allow_all"
weight = 100
enabled = true
# No options required
```

### Deny All

```toml
[[middleware.maintenance.options.policies.rules]]
type = "deny_all"
weight = 0
enabled = true
# No options required
```

## Policy Rules Examples

Complete policy configurations that combine several rule types. See [Policy Rules Examples](policy-rules-examples.md).

## See Also

- [Policies Middleware Documentation](policies-middleware.md) - Complete documentation with evaluation logic
- [Configuration Guide](configuration-file.md) - Overall configuration structure
- [Middleware Overview](middleware.md) - Other middleware types
