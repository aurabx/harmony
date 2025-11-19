# Policy Rules Quick Reference

**Last Updated**: 2025-01-14

This document provides a quick reference for all available policy rule types in the Harmony Proxy policies middleware.

## Table of Contents

- [Overview](#overview)
- [Rule Types Summary](#rule-types-summary)
- [IP-Based Rules](#ip-based-rules)
- [Path-Based Rules](#path-based-rules)
- [Header-Based Rules](#header-based-rules)
- [Geographic Rules](#geographic-rules)
- [Rate Limiting Rules](#rate-limiting-rules)
- [Time-Based Rules](#time-based-rules)
- [HTTP Method Rules](#http-method-rules)
- [User Agent Rules](#user-agent-rules)
- [Content Type Rules](#content-type-rules)
- [Query Parameter Rules](#query-parameter-rules)
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

## Time-Based Rules

### Business Hours

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

### Maintenance Window

```toml
[[middleware.maintenance.options.policies.rules]]
type = "time_based"
weight = 100
enabled = true

[middleware.maintenance.options.policies.rules.options]
allow_during_window = false   # Deny during window
timezone = "UTC"
start_time = "02:00"
end_time = "04:00"
```

### Date Range

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

## HTTP Method Rules

### Allow Only Read Operations

```toml
[[middleware.api_methods.options.policies.rules]]
type = "method"
weight = 85
enabled = true

[middleware.api_methods.options.policies.rules.options]
mode = "allow"
methods = ["GET", "HEAD", "OPTIONS"]
```

### Block Destructive Operations

```toml
[[middleware.api_methods.options.policies.rules]]
type = "method"
weight = 90
enabled = true

[middleware.api_methods.options.policies.rules.options]
mode = "deny"
methods = ["DELETE", "PUT"]
```

### Allow Standard REST Methods

```toml
[[middleware.api_methods.options.policies.rules]]
type = "method"
weight = 85
enabled = true

[middleware.api_methods.options.policies.rules.options]
mode = "allow"
methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
```

## User Agent Rules

### Block Bots and Scrapers

```toml
[[middleware.bot_filter.options.policies.rules]]
type = "user_agent"
weight = 75
enabled = true

[middleware.bot_filter.options.policies.rules.options]
mode = "deny"
patterns = [
    { label = "Common Bots", pattern = "/bot|crawler|spider/i" },
    { label = "Scrapers", pattern = "/scrapy|selenium/i" },
    { label = "Python Requests", pattern = "/^python-requests/i" }
]
```

### Allow Only Modern Browsers

```toml
[[middleware.browser_filter.options.policies.rules]]
type = "user_agent"
weight = 75
enabled = true

[middleware.browser_filter.options.policies.rules.options]
mode = "allow"
patterns = [
    { label = "Chrome", pattern = "/Chrome/i" },
    { label = "Firefox", pattern = "/Firefox/i" },
    { label = "Safari", pattern = "/Safari/i" },
    { label = "Edge", pattern = "/Edg/i" }
]
```

### Filter Mobile Devices

```toml
[[middleware.mobile_filter.options.policies.rules]]
type = "user_agent"
weight = 75
enabled = true

[middleware.mobile_filter.options.policies.rules.options]
mode = "allow"
patterns = [
    { label = "Mobile Devices", pattern = "/Mobile|Android|iPhone|iPad/i" }
]
```

## Content Type Rules

### Allow JSON Only

```toml
[[middleware.content_filter.options.policies.rules]]
type = "content_type"
weight = 70
enabled = true

[middleware.content_filter.options.policies.rules.options]
mode = "allow"
content_types = ["application/json"]
```

### Allow Common Web Formats

```toml
[[middleware.content_filter.options.policies.rules]]
type = "content_type"
weight = 70
enabled = true

[middleware.content_filter.options.policies.rules.options]
mode = "allow"
content_types = [
    "application/json",
    "application/x-www-form-urlencoded",
    "multipart/form-data"
]
```

### Block XML for Security

```toml
[[middleware.content_filter.options.policies.rules]]
type = "content_type"
weight = 75
enabled = true

[middleware.content_filter.options.policies.rules.options]
mode = "deny"
content_types = [
    "application/xml",
    "text/xml"
]
```

### Wildcard Matching

```toml
[[middleware.content_filter.options.policies.rules]]
type = "content_type"
weight = 70
enabled = true

[middleware.content_filter.options.policies.rules.options]
mode = "allow"
content_types = [
    "application/*",   # All application types
    "text/plain"      # Plus plain text
]
```

## Query Parameter Rules

### Require API Key (Exists)

```toml
[[middleware.param_filter.options.policies.rules]]
type = "query_parameter"
weight = 90
enabled = true

[middleware.param_filter.options.policies.rules.options]
mode = "allow"
parameters = [
    { name = "api_key", match_type = "exists" }
]
```

### Exact Value Matching

```toml
[[middleware.param_filter.options.policies.rules]]
type = "query_parameter"
weight = 85
enabled = true

[middleware.param_filter.options.policies.rules.options]
mode = "allow"
parameters = [
    { name = "version", match_type = "exact", value = "v2" },
    { name = "format", match_type = "exact", value = "json" }
]
```

### Contains Matching

```toml
[[middleware.param_filter.options.policies.rules]]
type = "query_parameter"
weight = 85
enabled = true

[middleware.param_filter.options.policies.rules.options]
mode = "deny"
parameters = [
    { name = "role", match_type = "contains", value = "admin" }
]
```

### Regex Pattern Matching

```toml
[[middleware.param_filter.options.policies.rules]]
type = "query_parameter"
weight = 80
enabled = true

[middleware.param_filter.options.policies.rules.options]
mode = "allow"
parameters = [
    { name = "id", match_type = "regex", value = "/^[0-9]+$/" },
    { name = "timestamp", match_type = "regex", value = "/^[0-9]{10,}$/" }
]
```

### Multiple Parameter Validation

```toml
[[middleware.param_filter.options.policies.rules]]
type = "query_parameter"
weight = 92
enabled = true

[middleware.param_filter.options.policies.rules.options]
mode = "allow"
parameters = [
    { name = "api_key", match_type = "exists" },
    { name = "timestamp", match_type = "regex", value = "/^[0-9]{10,}$/" },
    { name = "action", match_type = "exact", value = "query" }
]
```

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

## Complete Examples

### Multi-Layered Security

```toml
[middleware.security]
type = "policies"

[[middleware.security.options.policies]]
id = "multi_layer_security"
name = "Multi-Layer Security Policy"
enabled = true

# Layer 1: Geographic restriction
[[middleware.security.options.policies.rules]]
type = "geo"
weight = 100
[middleware.security.options.policies.rules.options]
mode = "allow"
country_codes = ["US", "GB", "CA"]

# Layer 2: IP allowlist
[[middleware.security.options.policies.rules]]
type = "ip_allow"
weight = 90
[middleware.security.options.policies.rules.options]
ip_addresses = ["10.0.0.0/8", "192.168.0.0/16"]

# Layer 3: Block admin paths
[[middleware.security.options.policies.rules]]
type = "path"
weight = 85
[middleware.security.options.policies.rules.options]
mode = "deny"
paths = ["/admin/{*path}"]

# Layer 4: Block bots
[[middleware.security.options.policies.rules]]
type = "user_agent"
weight = 80
[middleware.security.options.policies.rules.options]
mode = "deny"
patterns = [{ pattern = "/bot|crawler|spider/i" }]

# Layer 5: Rate limiting
[[middleware.security.options.policies.rules]]
type = "rate_limit"
weight = 50
[middleware.security.options.policies.rules.options]
max_requests = 100
window_seconds = 60
```

### Read-Only API

```toml
[middleware.readonly_api]
type = "policies"

[[middleware.readonly_api.options.policies]]
id = "readonly"
enabled = true

# Allow only GET and HEAD
[[middleware.readonly_api.options.policies.rules]]
type = "method"
weight = 100
[middleware.readonly_api.options.policies.rules.options]
mode = "allow"
methods = ["GET", "HEAD"]

# Allow only JSON responses
[[middleware.readonly_api.options.policies.rules]]
type = "content_type"
weight = 90
[middleware.readonly_api.options.policies.rules.options]
mode = "allow"
content_types = ["application/json"]

# Require API key
[[middleware.readonly_api.options.policies.rules]]
type = "query_parameter"
weight = 95
[middleware.readonly_api.options.policies.rules.options]
mode = "allow"
parameters = [
    { name = "api_key", match_type = "exists" }
]
```

### Business Hours API

```toml
[middleware.business_hours]
type = "policies"

[[middleware.business_hours.options.policies]]
id = "hours"
enabled = true

# Business hours only
[[middleware.business_hours.options.policies.rules]]
type = "time_based"
weight = 100
[middleware.business_hours.options.policies.rules.options]
allow_during_window = true
timezone = "America/New_York"
start_time = "09:00"
end_time = "17:00"
days_of_week = ["monday", "tuesday", "wednesday", "thursday", "friday"]

# Allow internal IPs anytime
[[middleware.business_hours.options.policies.rules]]
type = "ip_allow"
weight = 90
[middleware.business_hours.options.policies.rules.options]
ip_addresses = ["10.0.0.0/8"]
```

## See Also

- [Policies Middleware Documentation](policies-middleware.md) - Complete documentation with evaluation logic
- [Configuration Guide](configuration.md) - Overall configuration structure
- [Middleware Overview](middleware.md) - Other middleware types
