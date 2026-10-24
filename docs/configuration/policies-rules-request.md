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

## 9. Time Based (`time_based`)

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

## 10. HTTP Method (`method`)

Allow or deny requests based on HTTP method (GET, POST, PUT, DELETE, etc.).

**Options:**
- `mode` (string, required): "allow" or "deny"
- `methods` (array of strings, required): HTTP methods to match

**Example:**
```toml
[[middleware.api_methods.options.policies.rules]]
type = "method"
weight = 85
enabled = true

[middleware.api_methods.options.policies.rules.options]
mode = "allow"
methods = ["GET", "POST"]
```

**Supported Methods:**
- `GET` - Retrieve data
- `POST` - Submit data
- `PUT` - Update/replace data
- `PATCH` - Partial update
- `DELETE` - Remove data
- `OPTIONS` - Get supported methods
- `HEAD` - Get headers only

**Behavior:**
- Extracts HTTP method from request metadata
- Matches against configured methods list
- On match + mode="allow": Rule evaluates to ALLOW
- On match + mode="deny": Rule evaluates to DENY
- On no match: Rule evaluates to NO_MATCH

**Use Cases:**
- Read-only API endpoints (GET only)
- Restrict destructive operations (deny DELETE)
- Separate read/write permissions
- Method-based routing control

## 11. User Agent (`user_agent`)

Allow or deny requests based on User-Agent header patterns using regex.

**Options:**
- `mode` (string, required): "allow" or "deny"
- `patterns` (array of objects, required): Pattern match configurations
  - `label` (string, optional): Friendly name for the pattern
  - `pattern` (string, required): Regex pattern to match

**Example:**
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
    { pattern = "/^python-requests/i" }
]
```

**Pattern Syntax:**
- Standard regex with delimiters: `/pattern/flags`
- Common flags:
  - `i` - Case-insensitive matching
  - `m` - Multiline matching
- Examples:
  - `/Mozilla.*Chrome/` - Match Chrome browsers
  - `/bot|crawler/i` - Match any bot or crawler (case-insensitive)
  - `/^Mobile|Android|iPhone/` - Match mobile devices

**Behavior:**
- Extracts User-Agent from request headers/metadata
- Tests against all configured patterns (any match triggers rule)
- Invalid regex patterns log warning and are skipped
- On any pattern match + mode="allow": Rule evaluates to ALLOW
- On any pattern match + mode="deny": Rule evaluates to DENY
- On no match: Rule evaluates to NO_MATCH

**Use Cases:**
- Block bots and scrapers
- Allow only specific browsers
- Filter by device type (mobile vs desktop)
- Detect and block automated tools
- API client version enforcement

## 12. Content Type (`content_type`)

Allow or deny requests based on Content-Type header.

**Options:**
- `mode` (string, required): "allow" or "deny"
- `content_types` (array of strings, required): MIME types to match (supports wildcards)

**Example:**
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

**Example with Wildcards:**
```toml
[[middleware.content_filter.options.policies.rules]]
type = "content_type"
weight = 70
enabled = true

[middleware.content_filter.options.policies.rules.options]
mode = "deny"
content_types = [
    "application/xml",     # Specific type
    "text/*"              # All text types
]
```

**Wildcard Support:**
- `application/*` - Matches any application type
- `text/*` - Matches any text type
- `image/*` - Matches any image type
- Exact matches also supported: `application/json`

**Common MIME Types:**
- `application/json` - JSON data
- `application/xml` - XML data
- `text/html` - HTML documents
- `text/plain` - Plain text
- `multipart/form-data` - File uploads
- `application/x-www-form-urlencoded` - Form submissions

**Behavior:**
- Extracts Content-Type from request headers
- Supports wildcard matching (e.g., `application/*`)
- On match + mode="allow": Rule evaluates to ALLOW
- On match + mode="deny": Rule evaluates to DENY
- On no match: Rule evaluates to NO_MATCH

**Use Cases:**
- Accept only JSON API requests
- Block XML uploads for security
- Restrict file upload types
- Content-type based routing
- API versioning by content type

## 13. Query Parameter (`query_parameter`)

Allow or deny requests based on query parameter values with flexible matching.

**Options:**
- `mode` (string, required): "allow" or "deny"
- `parameters` (array of objects, required): Parameter match configurations
  - `name` (string, required): Query parameter name
  - `match_type` (string, required): "exists", "exact", "contains", or "regex"
  - `value` (string, optional): Value or pattern to match (not required for "exists")

**Example 1: Require API Key**
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

**Example 2: Exact Value Match**
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

**Example 3: Regex Pattern**
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

**Example 4: Block Admin Access**
```toml
[[middleware.param_filter.options.policies.rules]]
type = "query_parameter"
weight = 95
enabled = true

[middleware.param_filter.options.policies.rules.options]
mode = "deny"
parameters = [
    { name = "role", match_type = "contains", value = "admin" }
]
```

**Match Types:**
- `exists`: Parameter must be present (any value, including empty)
- `exact`: Parameter value must match exactly (case-sensitive)
- `contains`: Parameter value must contain the specified text (case-sensitive)
- `regex`: Parameter value must match regex pattern

**Behavior:**
- Extracts query parameters from request
- ALL configured parameters must match for rule to trigger
- Invalid regex patterns log warning and return false
- On all match + mode="allow": Rule evaluates to ALLOW
- On all match + mode="deny": Rule evaluates to DENY
- On any no match: Rule evaluates to NO_MATCH

**Use Cases:**
- Require API keys or tokens
- Validate request parameters
- Filter by version or format
- Block requests with suspicious parameters
- Enforce parameter patterns (numeric IDs, dates, etc.)
- API access control based on flags
