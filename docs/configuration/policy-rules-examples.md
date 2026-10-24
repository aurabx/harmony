---
title: Policy Rules Examples
description: Complete policy configurations that combine several rule types.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/policy-rules-reference.md
---

# Policy Rules Examples

Complete policy configurations that combine several rule types. This page is part of [Policy Rules Quick Reference](policy-rules-reference.md).

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
