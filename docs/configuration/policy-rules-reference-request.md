---
title: Policy Rules Reference: Request Attributes
description: Quick reference for the time, method, user agent, content type, and query parameter rules.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/policy-rules-reference.md
---

# Policy Rules Reference: Request Attributes

Quick reference for the time, method, user agent, content type, and query parameter rules. This page is part of [Policy Rules Quick Reference](policy-rules-reference.md).

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
