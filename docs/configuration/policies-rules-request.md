---
title: Policy Rules: Request Attributes
description: Policy rule types that match on time, method, user agent, content type, and query parameters.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/policies-middleware.md
---

# Policy Rules: Request Attributes

Policy rule types that match on time, method, user agent, content type, and query parameters. This page is part of [Policies Middleware](policies-middleware.md).

Each example defines a rule as a top-level `[rules.<id>]` table. Add the rule id to a policy's `rules` list to use it. See [Configuration Structure](policies-middleware.md#configuration-structure).

## 9. Time Based (`time_based`)

Allow or deny requests based on a time window, days of the week, and a date range.

**Options:**
- `allow_during_window` (boolean, default: true): If true, allow inside the window and deny outside it. If false, deny inside the window and allow outside it.
- `timezone` (string, default: `"UTC"`): IANA timezone, such as `"America/New_York"`
- `start_time` and `end_time` (strings, optional): `HH:MM`, 24-hour. Set both, or the time check is skipped.
- `days_of_week` (array of strings, optional): Three-letter day names: `"mon"`, `"tue"`, `"wed"`, `"thu"`, `"fri"`, `"sat"`, `"sun"`. Case does not matter. Full names such as `"monday"` never match. See [Known Issues](policies-middleware.md#known-issues).
- `start_date` and `end_date` (strings, optional): `YYYY-MM-DD`. Set both, or the date check is skipped.

**Example 1: Business Hours**
```toml
[rules.business_hours]
id = "business_hours"
type = "time_based"
weight = 100
[rules.business_hours.options]
allow_during_window = true
timezone = "America/New_York"
start_time = "09:00"
end_time = "17:00"
days_of_week = ["mon", "tue", "wed", "thu", "fri"]
```

**Example 2: Maintenance Window**
```toml
[rules.maintenance_window]
id = "maintenance_window"
type = "time_based"
weight = 100
[rules.maintenance_window.options]
allow_during_window = false  # Deny during the window
timezone = "UTC"
start_time = "02:00"
end_time = "04:00"
```

This rule returns ALLOW at every time outside 02:00 to 04:00. If it is the only allow rule in the middleware, it admits every request outside the window.

**Example 3: Date Range**
```toml
[rules.event_dates]
id = "event_dates"
type = "time_based"
weight = 100
[rules.event_dates.options]
allow_during_window = true
timezone = "UTC"
start_date = "2025-01-01"
end_date = "2025-12-31"
```

**Behavior:**
- Uses the current time and day in `timezone`
- Checks the time window. A window that crosses midnight, such as 22:00 to 06:00, is supported.
- Checks the day of week. The current day must be in the list.
- Checks the date range. Both dates are included.
- Inside the window on every configured check: ALLOW if `allow_during_window = true`, else DENY (403)
- Outside the window on any check: DENY (403) if `allow_during_window = true`, else ALLOW
- An invalid timezone logs a warning and uses UTC
- An invalid time or date logs a warning and the rule returns no match

## 10. HTTP Method (`method`)

Allow or deny requests by HTTP method.

**Options:**
- `mode` (string, default: `"deny"`): `"allow"` or `"deny"`
- `methods` (array of strings, required): Methods to match. Case does not matter.

**Example: allow read operations only**
```toml
[rules.read_only_methods]
id = "read_only_methods"
type = "method"
weight = 85
[rules.read_only_methods.options]
mode = "allow"
methods = ["GET", "HEAD", "OPTIONS"]
```

**Example: block destructive operations**
```toml
[rules.block_destructive]
id = "block_destructive"
type = "method"
weight = 90
[rules.block_destructive.options]
mode = "deny"
methods = ["DELETE", "PUT"]
```

**Example: standard REST methods**
```toml
[rules.rest_methods]
id = "rest_methods"
type = "method"
weight = 85
[rules.rest_methods.options]
mode = "allow"
methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
```

**Behavior:**
- Reads the method from the request
- On match + `mode = "allow"`: ALLOW
- On no match + `mode = "allow"`: DENY (405)
- On match + `mode = "deny"`: DENY (405)
- On no match + `mode = "deny"`: no match
- A missing or empty `methods` list: no match

## 11. User Agent (`user_agent`)

Allow or deny requests by matching the `User-Agent` header against regex patterns.

**Options:**
- `mode` (string, default: `"deny"`): `"allow"` or `"deny"`
- `patterns` (array of tables, required): Patterns to match
  - `label` (string, optional): Name shown in debug logs
  - `pattern` (string, required): Regex pattern

**Example: block bots and scrapers**
```toml
[rules.block_bots]
id = "block_bots"
type = "user_agent"
weight = 75
[rules.block_bots.options]
mode = "deny"
patterns = [
    { label = "Common Bots", pattern = "/bot|crawler|spider/i" },
    { label = "Scrapers", pattern = "/scrapy|selenium/i" },
    { label = "Python Requests", pattern = "/^python-requests/i" }
]
```

**Example: allow common browsers**
```toml
[rules.browsers_only]
id = "browsers_only"
type = "user_agent"
weight = 75
[rules.browsers_only.options]
mode = "allow"
patterns = [
    { label = "Chrome", pattern = "/Chrome/i" },
    { label = "Firefox", pattern = "/Firefox/i" },
    { label = "Safari", pattern = "/Safari/i" },
    { label = "Edge", pattern = "/Edg/i" }
]
```

**Example: allow mobile devices**
```toml
[rules.mobile_only]
id = "mobile_only"
type = "user_agent"
weight = 75
[rules.mobile_only.options]
mode = "allow"
patterns = [
    { label = "Mobile Devices", pattern = "/Mobile|Android|iPhone|iPad/i" }
]
```

An allow-mode `user_agent` rule returns no match for other clients. It does not deny them. They are denied only if no other rule allows the request.

**Pattern Syntax:**
- Write a pattern as `/pattern/flags` or as a bare pattern
- Only the `i` flag (ignore case) has an effect. Other flags are ignored.
- Patterns use Rust regex syntax

**Behavior:**
- Reads the `user-agent` entry of the request headers
- No `User-Agent` header: no match
- Any pattern matches: the rule matches
- An invalid pattern logs a warning and is skipped
- On match + `mode = "allow"`: ALLOW
- On match + `mode = "deny"`: DENY (403)
- On no match: no match

## 12. Content Type (`content_type`)

Allow or deny requests by the `Content-Type` header.

**Options:**
- `mode` (string, default: `"deny"`): `"allow"` or `"deny"`
- `content_types` (array of strings, required): MIME types. A `type/*` entry matches any subtype.

**Example: allow common API formats**
```toml
[rules.api_formats]
id = "api_formats"
type = "content_type"
weight = 70
[rules.api_formats.options]
mode = "allow"
content_types = [
    "application/json",
    "application/x-www-form-urlencoded",
    "multipart/form-data"
]
```

**Example: block XML**
```toml
[rules.block_xml]
id = "block_xml"
type = "content_type"
weight = 75
[rules.block_xml.options]
mode = "deny"
content_types = ["application/xml", "text/xml"]
```

**Example: wildcard**
```toml
[rules.application_types]
id = "application_types"
type = "content_type"
weight = 70
[rules.application_types.options]
mode = "allow"
content_types = [
    "application/*",  # Any application subtype
    "text/plain"
]
```

**Behavior:**
- Reads the `content-type` entry of the request headers
- Removes parameters such as `; charset=utf-8` before matching
- Matching is case-sensitive
- `"*/*"` never matches. See [Known Issues](policies-middleware.md#known-issues).
- No `Content-Type` header: no match, in both modes
- On match + `mode = "allow"`: ALLOW
- On no match + `mode = "allow"`: DENY (415)
- On match + `mode = "deny"`: DENY (415)
- On no match + `mode = "deny"`: no match

## 13. Query Parameter (`query_parameter`)

Allow or deny requests by query parameter values.

By default this rule does not see HTTP query parameters. In allow mode it then denies every HTTP request. See [Known Issues](policies-middleware.md#known-issues).

**Options:**
- `mode` (string, default: `"deny"`): `"allow"` or `"deny"`
- `parameters` (array of tables, required): Parameter checks
  - `name` (string, required): Parameter name. An entry without a name is skipped.
  - `match_type` (string, default: `"exact"`): `"exists"`, `"exact"`, `"contains"`, or `"regex"`
  - `value` (string, default: empty): Value or pattern. Not used by `"exists"`.

**Example: block an admin role**
```toml
[rules.block_admin_role]
id = "block_admin_role"
type = "query_parameter"
weight = 95
[rules.block_admin_role.options]
mode = "deny"
parameters = [
    { name = "role", match_type = "contains", value = "admin" }
]
```

**Example: check several parameters**
```toml
[rules.signed_query]
id = "signed_query"
type = "query_parameter"
weight = 92
[rules.signed_query.options]
mode = "allow"
parameters = [
    { name = "api_key", match_type = "exists" },
    { name = "timestamp", match_type = "regex", value = "^[0-9]{10,}$" },
    { name = "action", match_type = "exact", value = "query" }
]
```

**Match Types:**
- `exists`: The parameter is present
- `exact`: Case-sensitive exact match
- `contains`: Case-sensitive substring match
- `regex`: Rust regex match. Write the pattern without `/…/` delimiters.

**Behavior:**
- Reads parameters from the `query_params` metadata key (`key1=value1&key2=value2`, not URL-decoded) and from metadata keys that start with `param_`
- Every configured parameter must match for the rule to match
- An invalid regex logs a warning and counts as a non-match
- On all match + `mode = "allow"`: ALLOW
- On any non-match + `mode = "allow"`: DENY (403)
- On all match + `mode = "deny"`: DENY (403)
- On any non-match + `mode = "deny"`: no match
