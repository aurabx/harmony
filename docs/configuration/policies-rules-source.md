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

Each example defines a rule as a top-level `[rules.<id>]` table. Add the rule id to a policy's `rules` list to use it. See [Configuration Structure](policies-middleware.md#configuration-structure).

## 1. IP Allow (`ip_allow`)

Allow requests from specific IP addresses or CIDR ranges.

**Options:**
- `ip_addresses` (array of strings, required): IP addresses or CIDR ranges. A bare address such as `203.0.113.45` is accepted. An invalid entry fails the config load.

**Example:**
```toml
[rules.internal_networks]
id = "internal_networks"
type = "ip_allow"
weight = 100
[rules.internal_networks.options]
ip_addresses = [
    "10.0.0.0/8",       # Private network - Class A
    "172.16.0.0/12",    # Private network - Class B
    "192.168.0.0/16",   # Private network - Class C
    "127.0.0.1/32"      # Localhost
]
```

**Behavior:**
- Reads the client IP from the `remote_addr` metadata key, then `client_ip`
- Checks the IP against each CIDR range in turn
- Acts as a required gate for the middleware: a client IP that matches no `ip_allow` rule in any policy is denied (403), even if another rule allows it
- `ip_allow` rules form a union, within a policy and across policies
- An unknown client IP is denied (403, fail closed)
- On match: ALLOW
- See [IP Allowlist Gate](policies-middleware.md#ip-allowlist-gate)

**Use Cases:**
- Internal network allowlisting
- VPN-only access
- Specific office IP ranges

## 2. IP Deny (`ip_deny`)

Block requests from specific IP addresses or CIDR ranges.

**Options:**
- `ip_addresses` (array of strings, required): IP addresses or CIDR ranges

**Example:**
```toml
[rules.blocklist]
id = "blocklist"
type = "ip_deny"
weight = 90
[rules.blocklist.options]
ip_addresses = [
    "203.0.113.0/24",   # TEST-NET-3 (example blocklist)
    "198.51.100.0/24"   # TEST-NET-2 (example blocklist)
]
```

**Behavior:**
- Same IP matching as `ip_allow`
- On match: DENY (403)
- On no match, or no parseable client IP: no match

**Use Cases:**
- Block known malicious IPs
- Emergency blocking of compromised IPs

## 3. Rate Limit (`rate_limit`)

Limit the number of requests each client IP can make in a time window.

**Options:**
- `max_requests` (integer, default: 100): Requests allowed in one window
- `window_seconds` (integer, default: 60): Window length in seconds

**Example:**
```toml
[rules.api_rate_limit]
id = "api_rate_limit"
type = "rate_limit"
weight = 50
[rules.api_rate_limit.options]
max_requests = 100
window_seconds = 60  # 100 requests per minute
```

**Behavior:**
- Uses a fixed window. The window starts at a client's first request. The first request after `window_seconds` have passed starts a new window with a count of 1.
- Counts are kept per `<client IP>:<policy index>:<rule index>`. Each rate-limit rule has its own count.
- The client IP comes from `remote_addr`, then `client_ip`. Harmony writes it in canonical form. An IPv4-mapped address (`::ffff:a.b.c.d`) shares a count with the plain IPv4 address. Different spellings of one IPv6 address share one count.
- Requests with no client IP all share one count under the name `unknown`.
- Within the limit: the count goes up by one and the rule returns no match.
- Over the limit: the rule returns DENY (429). Denied requests do not add to the count.
- Every enabled rule is evaluated, so a request that another rule denies still uses up rate-limit budget. A client outside the IP allowlist gate does not.
- Pipelines that use the same middleware instance share counts.

**Important:** A rate-limit rule never returns ALLOW. The policy needs another rule that returns ALLOW, such as `allow_all` or `ip_allow`.

**Use Cases:**
- API rate limiting (100 requests per minute)
- Fair usage enforcement

Windows longer than 300 seconds reset early. See [Known Issues](policies-middleware.md#known-issues).

## 4. Path Match (`path`)

Allow or deny based on the request path.

**Options:**
- `paths` (array of strings, required): Path patterns in matchit syntax. Each must start with `/`.
- `mode` (string, required): `"allow"` or `"deny"`. Any other value fails the config load.

**Example:**
```toml
[rules.public_paths]
id = "public_paths"
type = "path"
weight = 80
[rules.public_paths.options]
mode = "allow"
paths = [
    "/api/public/{*path}",  # Everything under /api/public/
    "/health",
    "/metrics"
]
```

**Pattern Syntax (matchit 0.8):**
- Exact: `/users`, `/api/health`
- Parameter: `/users/{id}` matches one segment, such as `/users/42`
- Catch-all: `/api/{*path}` matches one or more characters after `/api/`. It does not match `/api` or `/api/`. It must be the last part of the pattern.
- `*` outside braces is a literal character. `/api/*` matches only the path `/api/*`.

**Behavior:**
- Reads the path from the `path` metadata key. For HTTP this is the path after the endpoint's `path_prefix`. A missing path is treated as `/`.
- Adds a leading `/` if the path lacks one
- Does not strip a trailing `/`. `/health/` does not match `/health`.
- On match + `mode = "allow"`: ALLOW
- On match + `mode = "deny"`: DENY (403)
- On no match: no match

**Use Cases:**
- Protect admin endpoints (`/admin/{*path}` deny)
- Allow only specific API paths

## 5. Geographic (`geo`)

Allow or deny based on the client's country code.

By default this rule does not match HTTP requests. Nothing in Harmony sets the metadata it reads. See [Known Issues](policies-middleware.md#known-issues).

**Options:**
- `country_codes` (array of strings, required): ISO 3166-1 alpha-2 codes. Case does not matter.
- `mode` (string, default: `"deny"`): `"allow"` or `"deny"`

**Example: allow some countries**
```toml
[rules.allowed_countries]
id = "allowed_countries"
type = "geo"
weight = 70
[rules.allowed_countries.options]
mode = "allow"
country_codes = ["US", "GB", "CA", "AU"]
```

**Example: block some countries**
```toml
[rules.blocked_countries]
id = "blocked_countries"
type = "geo"
weight = 70
[rules.blocked_countries.options]
mode = "deny"
country_codes = ["XX", "YY"]  # Replace with real ISO 3166-1 alpha-2 codes
```

**Behavior:**
- Reads the country from the `geo_country` metadata key, then `country_code`
- Compares codes in upper case
- No country in the metadata: no match
- On match + `mode = "allow"`: ALLOW
- On match + `mode = "deny"`: DENY (403)
- On no match: no match

## 6. Header Match (`header`)

Allow or deny based on header values.

By default this rule does not match HTTP requests. Nothing in Harmony sets the metadata keys it reads. See [Known Issues](policies-middleware.md#known-issues).

**Options:**
- `mode` (string, default: `"deny"`): `"allow"` or `"deny"`
- `headers` (array of tables, required): Header checks
  - `name` (string): Header name
  - `match_type` (string, default: `"exact"`): `"exact"`, `"contains"`, or `"regex"`
  - `value` (string): Value or pattern to match. An entry without `name` or `value` is skipped.

**Example: require an exact value**
```toml
[rules.api_key_header]
id = "api_key_header"
type = "header"
weight = 60
[rules.api_key_header.options]
mode = "allow"
headers = [
    { name = "X-API-Key", match_type = "exact", value = "secret-key" }
]
```

**Example: block a header that contains a value**
```toml
[rules.block_debug_header]
id = "block_debug_header"
type = "header"
weight = 60
[rules.block_debug_header.options]
mode = "deny"
headers = [
    { name = "X-Debug", match_type = "contains", value = "true" }
]
```

**Match Types:**
- `exact`: Exact match, ignoring ASCII case
- `contains`: Substring match, ignoring case
- `regex`: Rust regex match. Write the pattern without `/…/` delimiters. Matching is case-sensitive unless the pattern starts with `(?i)`.

**Behavior:**
- Reads each header from the metadata key `header_<lower-case name>`, then the key `<name>`
- Every configured header must match for the rule to match
- An invalid regex logs a warning and counts as a non-match
- On all match + `mode = "allow"`: ALLOW
- On all match + `mode = "deny"`: DENY (403)
- On any non-match: no match

## 7. Allow All (`allow_all`)

Always returns ALLOW. It takes no options.

**Example:**
```toml
[rules.allow_everyone]
id = "allow_everyone"
type = "allow_all"
```

**Use Cases:**
- The base allow rule for a policy that only denies, such as rate limiting
- Public endpoints

## 8. Deny All (`deny_all`)

Always returns DENY (403). It takes no options.

**Example:**
```toml
[rules.maintenance_block]
id = "maintenance_block"
type = "deny_all"
```

**Use Cases:**
- Maintenance mode
- Emergency blocking
