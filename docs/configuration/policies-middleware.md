---
title: Policies Middleware
description: How to configure the policies middleware, its evaluation logic, and rule weighting.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/policies-rules-source.md
  - configuration/policies-rules-request.md
  - configuration/policies-patterns.md
---

# Policies Middleware

The policies middleware admits or rejects each incoming request. It checks the request against rules that you group into policies. It runs on incoming requests only. It does nothing to responses.

## Table of Contents

- [Overview](#overview)
- [Rule Types Summary](#rule-types-summary)
- [Evaluation Logic](#evaluation-logic)
- [Configuration Structure](#configuration-structure)
- [Rule Types](#rule-types)
- [Weight and Priority](#weight-and-priority)
- [Common Patterns](policies-patterns.md#common-patterns)
- [Best Practices](policies-patterns.md#best-practices)
- [Performance](#performance)
- [Known Issues](#known-issues)
- [Troubleshooting](policies-patterns.md#troubleshooting)

## Overview

Use the policies middleware to:
- Allow or block clients by IP address or CIDR range
- Limit the request rate per client IP
- Allow or block requests by path, HTTP method, User-Agent, or Content-Type
- Allow or block requests by time of day, day of week, or date range

The middleware has 13 rule types. Every enabled rule is evaluated on every request. By default, header, query parameter, and geographic rules do not see HTTP request data. See [Known Issues](#known-issues).

## Rule Types Summary

Each rule returns ALLOW, DENY, or no match. "Mode" is the `mode` option. A rule with a mode defaults to `"deny"` if you leave it out, except `path`, which requires it.

| Type | Matches on | Mode | Options | Page |
|------|------------|------|---------|------|
| `ip_allow` | Client IP | none | `ip_addresses` | [Source](policies-rules-source.md#1-ip-allow-ip_allow) |
| `ip_deny` | Client IP | none | `ip_addresses` | [Source](policies-rules-source.md#2-ip-deny-ip_deny) |
| `rate_limit` | Requests per client IP | none | `max_requests`, `window_seconds` | [Source](policies-rules-source.md#3-rate-limit-rate_limit) |
| `path` | Request path | required | `paths`, `mode` | [Source](policies-rules-source.md#4-path-match-path) |
| `geo` | Country code metadata | default deny | `country_codes`, `mode` | [Source](policies-rules-source.md#5-geographic-geo) |
| `header` | Header metadata | default deny | `headers`, `mode` | [Source](policies-rules-source.md#6-header-match-header) |
| `allow_all` | Every request | none | none | [Source](policies-rules-source.md#7-allow-all-allow_all) |
| `deny_all` | Every request | none | none | [Source](policies-rules-source.md#8-deny-all-deny_all) |
| `time_based` | Current time and date | none | `allow_during_window`, `timezone`, `start_time`, `end_time`, `days_of_week`, `start_date`, `end_date` | [Request](policies-rules-request.md#9-time-based-time_based) |
| `method` | HTTP method | default deny | `methods`, `mode` | [Request](policies-rules-request.md#10-http-method-method) |
| `user_agent` | `User-Agent` header | default deny | `patterns`, `mode` | [Request](policies-rules-request.md#11-user-agent-user_agent) |
| `content_type` | `Content-Type` header | default deny | `content_types`, `mode` | [Request](policies-rules-request.md#12-content-type-content_type) |
| `query_parameter` | Query parameter metadata | default deny | `parameters`, `mode` | [Request](policies-rules-request.md#13-query-parameter-query_parameter) |

An unknown `type` is not rejected at load. At request time the rule logs `Unknown rule type '<type>' - treating as no match` at `warn` and never matches.

## Evaluation Logic

**A request is ACCEPTED only if:**
1. It passes the IP allowlist gate, if any policy has an `ip_allow` rule (see [IP Allowlist Gate](#ip-allowlist-gate))
2. AND at least one rule returns ALLOW, across all enabled policies
3. AND no rule returns DENY

### Key Points

- **IP allowlists are a required gate.** A client that matches no `ip_allow` rule in any policy is denied, whatever other rules allow.
- **All rules are evaluated.** Evaluation does not stop at the first match.
- **Implicit deny.** If no rule returns ALLOW, the request is denied with 403.
- **Deny wins.** One DENY blocks the request, whatever the allow rules say.
- **Some rules deny on a non-match.** `method`, `content_type` and `query_parameter` in allow mode return DENY when the request does not match. `time_based` returns DENY outside its window when `allow_during_window = true`.

### Evaluation Flow

```
1. IP allowlist gate. If any enabled policy has enabled ip_allow rules:
   - Client IP unknown or unparseable → DENY (403, fail closed)
   - Client IP matches no ip_allow rule in any policy → DENY (403)
2. For each enabled policy, in the order the middleware lists them:
   a. Take the enabled rules
   b. Sort them by weight, highest first
   c. Evaluate every rule
   d. Record whether any rule returned ALLOW, and the latest rule that returned DENY
3. Final decision:
   - A rule returned DENY → reject with that rule's status code
   - No rule returned ALLOW → reject with 403 (implicit deny)
   - Otherwise → continue to the next middleware
```

### Status Codes

The HTTP adapter maps the middleware's errors to these status codes (`src/adapters/http/router.rs`, `map_pipeline_error_to_status`):

| Denied by | Status |
|-----------|--------|
| `rate_limit` | 429 Too Many Requests |
| `method` | 405 Method Not Allowed |
| `content_type` | 415 Unsupported Media Type |
| Any other rule type | 403 Forbidden |
| IP allowlist gate | 403 Forbidden |
| No ALLOW (implicit deny) | 403 Forbidden |

When several rules return DENY, the status comes from the last one evaluated. That is the lowest-weight DENY in the last policy that denied. See [Weight and Priority](#weight-and-priority).

### IP Allowlist Gate

An `ip_allow` rule restricts who may use the pipeline. Other allow rules cannot widen it.

- **One gate for the middleware.** If any policy in the middleware has an enabled `ip_allow` rule, the client IP must match at least one `ip_allow` rule. The match can be in any policy.
- **Allowlists form a union.** Several `ip_allow` rules in one policy form a union. So do `ip_allow` rules in different policies. "Office OR VPN" can be two rules in one policy or two policies.
- **Fail closed.** If an allowlist exists and the client IP is unknown, the request is denied. This covers a missing `remote_addr` and a value that is not an IP address.
- **Status.** A gate denial returns 403 Forbidden, the same status as an `ip_deny` match.
- **Other rules still apply.** A request that passes the gate still needs an allow and no deny from the other rules. `ip_deny`, `path`, `method`, `rate_limit` and the rest behave as before.
- **Evaluated first.** The gate runs before the other rules. A client outside the allowlist does not use any rate-limit budget.
- **No `ip_allow` rules, no gate.** If no policy has an `ip_allow` rule, one matching allow rule and no deny still admits the request.
- **Logs.** An outside client is logged at `info` with the client IP and the ids of every `ip_allow` rule checked. An unknown client IP is logged at `warn`.

The client IP comes from the envelope's `remote_addr` metadata (then `client_ip`). The HTTP adapter sets `remote_addr` from the TCP peer address for HTTP/1.1 and HTTP/2. The HTTP/3 adapter sets it from the QUIC peer address. The DIMSE adapter sets it from the TCP peer address of the SCU's association, so the same rules apply to a `dicom_scp` endpoint. None of them reads `X-Forwarded-For`. HTTP/3 listeners turn QUIC connection migration off, so `remote_addr` is always the address the handshake checked. A client whose IP changes must open a new connection.

**DIMSE associations.** A `dicom_scp` pipeline checks IP rules twice.

- **When the association opens.** The SCP checks the peer address before it accepts the A-ASSOCIATE-RQ. It uses the `ip_deny` rules and the `ip_allow` gate of every `policies` middleware on the pipeline's incoming chain. A denied peer gets an A-ASSOCIATE-RJ (rejected-permanent, no reason given). It cannot send any command, so it gets no C-ECHO reply.
- **On each request.** C-FIND, C-GET, C-MOVE and C-STORE still run the whole middleware, with every rule type.
- **Only IP rules at association time.** Rules such as `method`, `path`, `header` and `rate_limit` have no meaning before a request exists, so they are skipped. A pipeline with no IP rules admits every peer.
- **Fail closed.** If the pipeline or one of its policies cannot be read, the association is rejected.
- **Shared listeners.** When several `dicom_scp` endpoints share one port, a peer may connect if any of their pipelines admits it. That includes a pipeline with no IP rules. C-ECHO has no pipeline of its own, so such a peer gets a C-ECHO reply. Each other request is checked by the pipeline that handles its operation.
- **Reloads.** The check reads the current config for each new association. Associations already open are not re-checked. A change to the listener itself, such as its port or `use_dcmtk_store`, restarts the network's DIMSE adapter. The restart stops a running `storescp` first.
- **DCMTK store.** If any endpoint on a port sets `use_dcmtk_store = true`, or sets `persistent_store_scp = true` (which turns `use_dcmtk_store` on by default), DCMTK `storescp` serves the whole port. `storescp` binds every interface and runs no IP check. A config is rejected at load or reload when any pipeline on that port has `ip_allow` or `ip_deny` rules. That includes pipelines on other networks or bind addresses that use the same port. A legacy `dicom_scu` backend with `persistent_store_scp = true` and `incoming_store_port` counts too, but only on a network that has a `dicom_scp` endpoint. Without one, no DIMSE adapter starts and the backend opens no listener. The error names the pipeline and the endpoint or backend that turned DCMTK on. To fix it, set `use_dcmtk_store = false` there, or remove the IP rules. Other rule types do not block DCMTK, but they never run on a DCMTK port either.
- **Logs.** A rejected peer is logged at `warn` with its address, the pipeline and the reason. If the check itself fails, for example because no config is loaded, the peer is rejected and the failure is logged at `error`.

**C-STORE copies.** On a `dicom_scp` endpoint, the DIMSE adapter writes its copy of a C-STORE instance to disk before the pipeline runs. It removes that copy after the store. A successful store through a pipeline with no backend is the exception: it keeps the copy as the stored instance. The adapter never keeps its own copy of a denied store. Put `policies` before `dimse_to_jmix` in the middleware chain. `dimse_to_jmix` moves the instance into its staging folder, and a later denial cannot undo that.

**IPv4-mapped addresses.** A dual-stack listener (`bind_address = "::"` with `IPV6_V6ONLY=0`, the default on Linux and macOS) reports an IPv4 client as an IPv4-mapped IPv6 address, such as `::ffff:10.1.2.3`. IP rules and rate limits treat that address as the plain IPv4 address `10.1.2.3`:

- An `ip_allow` or `ip_deny` CIDR written in IPv4, such as `10.0.0.0/8`, matches the mapped form.
- A CIDR written as an IPv6 network still matches the mapped form as received. `::ffff:10.0.0.0/104` keeps working.
- `remote_addr` itself is not changed. Logs and other middleware see the address as the listener reported it.
- Only `::ffff:0:0/96` is folded to IPv4. Other addresses that embed an IPv4 address, such as 6to4 (`2002::/16`), Teredo (`2001::/32`) and NAT64 (`64:ff9b::/96`), stay IPv6. An IPv4 `ip_deny` CIDR does not block them. Add the IPv6 ranges to the rule if you need to.

**Example:**

```toml
[policies.http_proxy_security]
id = "http_proxy_security"
rules = ["internal_networks", "method_filter"]

[rules.internal_networks]
id = "internal_networks"
type = "ip_allow"
[rules.internal_networks.options]
ip_addresses = ["10.0.0.0/8", "127.0.0.1/32"]

[rules.method_filter]
id = "method_filter"
type = "method"
[rules.method_filter.options]
mode = "allow"
methods = ["GET", "POST", "PUT", "DELETE"]
```

| Client IP | Method | Result |
|-----------|--------|--------|
| 10.1.2.3 | GET | ALLOW |
| 10.1.2.3 | PATCH | DENY 405 (method rule) |
| 203.0.113.9 | GET | DENY 403 (outside the allowlist) |
| unknown (no `remote_addr`) | GET | DENY 403 (fail closed) |

Before AURA-2438, the method rule's allow admitted 203.0.113.9, so the allowlist blocked nothing.

**Example with two policies (office OR VPN):**

```toml
[policies.office]
id = "office"
rules = ["office_networks", "method_filter"]

[policies.vpn]
id = "vpn"
rules = ["vpn_networks", "method_filter"]

[rules.office_networks]
id = "office_networks"
type = "ip_allow"
[rules.office_networks.options]
ip_addresses = ["192.168.0.0/16"]

[rules.vpn_networks]
id = "vpn_networks"
type = "ip_allow"
[rules.vpn_networks.options]
ip_addresses = ["10.8.0.0/16"]

[rules.method_filter]
id = "method_filter"
type = "method"
[rules.method_filter.options]
mode = "allow"
methods = ["GET"]
```

| Client IP | Method | Result |
|-----------|--------|--------|
| 192.168.1.5 | GET | ALLOW (office allowlist) |
| 10.8.0.7 | GET | ALLOW (VPN allowlist) |
| 203.0.113.9 | GET | DENY 403 (matches neither allowlist) |

## Configuration Structure

A `policies` middleware lists policy ids. Each policy is a top-level `[policies.<id>]` table that lists rule ids. Each rule is a top-level `[rules.<id>]` table. A rule can belong to more than one policy.

```toml
[middleware.my_policy]
type = "policies"
[middleware.my_policy.options]
policies = ["access_control"]

[policies.access_control]
id = "access_control"
name = "Access Control Policy"
enabled = true
rules = ["allow_internal", "deny_blocklist"]

[rules.allow_internal]
id = "allow_internal"
name = "Allow Internal Networks"
type = "ip_allow"
weight = 100
enabled = true
[rules.allow_internal.options]
ip_addresses = ["10.0.0.0/8", "192.168.0.0/16"]

[rules.deny_blocklist]
id = "deny_blocklist"
name = "Deny Blocklist"
type = "ip_deny"
weight = 90
[rules.deny_blocklist.options]
ip_addresses = ["203.0.113.0/24"]
```

Inline policy tables such as `[[middleware.my_policy.options.policies]]` are not accepted. The config fails to load with `All entries in 'policies' must be string IDs`.

### Configuration Fields

**Middleware options:**
- `policies` (array of strings, required): Policy ids. The array must not be empty.

**Policy fields (`[policies.<id>]`):**
- `id` (string, required): The policy id. Use the same value as the table key.
- `name` (string, optional): Human-readable name, used in logs
- `enabled` (boolean, default: true): Whether the policy is active
- `rules` (array of strings, default: empty): Rule ids

**Rule fields (`[rules.<id>]`):**
- `id` (string, required): The rule id. Use the same value as the table key.
- `name` (string, optional): Human-readable name. Deny logs and error messages show the name, or `unnamed` if it is not set.
- `type` (string, required): Rule type (see [Rule Types Summary](#rule-types-summary))
- `weight` (integer, default: 0): Evaluation order within the policy
- `enabled` (boolean, default: true): Whether the rule is active
- `options` (table, default: empty): Rule-specific settings

Disabled policies and rules are skipped. If every listed policy is disabled, the middleware fails to build with `Policies middleware requires at least one policy`.

### Undefined Rule and Policy Ids

Harmony checks rule and policy ids when it loads the config.

- Every id in a policy's `rules` list must be a key in `[rules]`.
- Every id in a `policies` middleware instance's `policies` list must be a key in `[policies]`.
- Disabled policies are checked too.

An undefined id fails the load. The error has one line per bad id. Each line names the policy or middleware instance and the undefined id. The lines are sorted. The defined ids follow, listed once for rules and once for policies:

```
Middleware 'access' references unknown policy 'missing_policy'
Policy 'office' references unknown rule 'missing_rule'
Available rules: internal_networks, method_filter
Available policies: office, vpn
```

The `Available rules` line appears only when a rule id is undefined. The `Available policies` line appears only when a policy id is undefined.

Control characters in a name or id are escaped in the error. A newline prints as `\n` and ESC prints as `\u{1b}`.

At startup and with `--validate-config`, Harmony prints the error on stderr and exits with code 1. On hot reload, Harmony rejects the file and keeps the previous config.


## Rule Types

### Policy Rules: Source and Access

Rule types that match on client address, rate, path, location, and headers. See [Policy Rules: Source and Access](policies-rules-source.md).

### Policy Rules: Request Attributes

Rule types that match on time, method, user agent, content type, and query parameters. See [Policy Rules: Request Attributes](policies-rules-request.md).

## Weight and Priority

Within each policy, rules run in weight order, highest first. Rules with the same weight keep their order in the policy's `rules` list. Policies run in the order the middleware lists them.

Weight does not change whether a request is admitted. Every enabled rule is evaluated, and any DENY blocks the request. Weight changes two things:

- **Log order.** Rules log their results in evaluation order.
- **Which deny is reported.** When several rules deny, the error and status code come from the last DENY evaluated. That is the lowest-weight DENY in the last policy that denied.

| Weight | Runs | Effect when it denies alongside other denies |
|--------|------|-----------------------------------------------|
| High (for example 100) | First | Its status is replaced by any later DENY |
| 0 (default) | Middle | Reported if no lower-weight rule denies |
| Low or negative (for example -1000) | Last | Its status is the one returned |

For example, a policy has a `method` rule (weight 100) and an `ip_deny` rule (weight 0). If both deny, the client gets 403 from `ip_deny`, not 405. Give the `method` rule the lower weight to report 405 instead.

## Policy Patterns and Troubleshooting

Common policy patterns, best practices, troubleshooting, and examples. See [Policy Patterns and Troubleshooting](policies-patterns.md).

## Performance

- **Rule compilation.** Harmony checks IP and path rules when it loads the config. The pipeline builds the middleware again for each request, which parses the CIDR ranges and builds the path router again.
- **IP rules.** Each `ip_allow` and `ip_deny` rule checks its CIDR ranges one by one.
- **Path rules.** Each `path` rule uses a matchit router.
- **Regex rules.** `header`, `user_agent` and `query_parameter` compile their regex patterns on every evaluation.
- **Rate limiting.** Counts live in memory, in a map behind an async `RwLock`. The map is shared by every pipeline that uses the same middleware instance name.
- **Rate-limit cleanup.** A background task runs every 60 seconds. It removes entries whose window started 300 seconds or more ago. See [Known Issues](#known-issues).

### Hot Reload Support

- Changes to `[policies]` and `[rules]` apply on reload without a restart.
- Rate-limit counts survive a reload, because they are keyed by middleware instance name.
- Counts are also keyed by the position of the policy and rule. If a reload reorders policies or rules, a count can move to a different rule.

## Known Issues

These describe the current code in `src/models/middleware/types/policies.rs`.

- **Header, query parameter, and geo rules do not see HTTP request data by default.**
  - `header` reads the metadata keys `header_<name>` and `<name>`.
  - `query_parameter` reads the metadata key `query_params` and keys starting with `param_`.
  - `geo` reads the metadata keys `geo_country` and `country_code`.
  - Nothing in Harmony sets these keys by default. A search of `src/` and `crates/` found no adapter or middleware that writes them. The HTTP adapter puts headers in `request_details.headers` and query parameters in `request_details.query_params`, which these rules do not read.
  - The result, with no middleware before `policies` that sets them: `header` and `geo` rules never match. A `query_parameter` rule in deny mode never matches. A `query_parameter` rule in allow mode denies every request with 403.
  - A middleware placed before `policies` can set these keys. A `metadata_transform` with `transform_target = "metadata"` writes the string values of its JOLT output into request metadata. Its input is the request metadata only, so it can set fixed values or copy other metadata keys. It cannot copy a header or query value. With the default `transform_target = "target_details"`, it writes to `target_details`, which these rules do not read.
  - A `transform` middleware sees the request headers and query parameters in its context. It merges string values from `context.request_details.metadata` in its output into request metadata. It runs only when the request has normalised data. This route has no test with `policies`.
- **`days_of_week` needs three-letter day names.** The rule compares against chrono's weekday name in lower case: `mon`, `tue`, `wed`, `thu`, `fri`, `sat`, `sun`. A full name such as `monday` never matches. With `allow_during_window = true`, a rule with full names denies every request.
- **`content_type = "*/*"` never matches.** The `type/*` wildcard check runs first and looks for the prefix `*`, which no real content type has.
- **Rate-limit windows longer than 300 seconds reset early.** Cleanup removes a count 300 to 360 seconds after its window started. A rule with `window_seconds = 3600` behaves like a window of about five minutes.
- **Unknown rule types are accepted.** A typo in `type` is not caught at load. The rule never matches, and logs a warning on each request.

## See Also

- [Configuration Guide](configuration-file.md) - Overall configuration structure
- [Middleware Overview](middleware.md) - Other middleware types
- [Path Filter Middleware](middleware.md#path-filter) - Simple path-based filtering
- [Hot Reload](../architecture/config-reload.md) - Configuration reload architecture
