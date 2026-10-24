---
title: Policy Rules: Source and Access
description: Policy rule types that match on client address, path, location, headers, and rate.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/policies-middleware.md
---

# Policy Rules: Source and Access

Policy rule types that match on client address, path, location, headers, and rate. This page is part of [Policies Middleware](policies-middleware.md).

## 1. IP Allow (`ip_allow`)

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
- Acts as a required gate for the middleware: a client IP that matches no `ip_allow` rule in any policy is denied (403), even if another rule allows it
- `ip_allow` rules form a union, within a policy and across policies
- An unknown client IP is denied (403, fail closed)
- On match: Rule evaluates to ALLOW
- See [IP Allowlist Gate](policies-middleware.md#ip-allowlist-gate)

**Use Cases:**
- Internal network allowlisting
- VPN-only access
- Specific office IP ranges

## 2. IP Deny (`ip_deny`)

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

## 3. Rate Limit (`rate_limit`)

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
- An IPv4-mapped address (`::ffff:a.b.c.d`) and the plain IPv4 address share one bucket. Different spellings of one IPv6 address, such as `2001:DB8:0::1` and `2001:db8::1`, also share one bucket
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

## 4. Path Match (`path`)

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

## 5. Geographic (`geo`)

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

## 6. Header Match (`header`)

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

## 7. Allow All (`allow_all`)

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

## 8. Deny All (`deny_all`)

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
