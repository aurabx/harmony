---
title: Policies Middleware
description: How to configure the policies middleware, its evaluation logic, and rule weighting.
status: active
updated: 2026-10-24
type: Configuration
---

# Policies Middleware

The policies middleware provides comprehensive policy-based access control, rate limiting, and request filtering through a flexible rule system.

## Table of Contents

- [Overview](#overview)
- [Evaluation Logic](#evaluation-logic)
- [Configuration Structure](#configuration-structure)
- [Rule Types](#rule-types)
- [Weight and Priority](#weight-and-priority)
- [Common Patterns](policies-patterns.md#common-patterns)
- [Best Practices](policies-patterns.md#best-practices)
- [Performance](#performance)
- [Troubleshooting](policies-patterns.md#troubleshooting)

## Overview

Policies middleware enables you to:
- Control access based on IP addresses, paths, headers, or geographic location
- Implement rate limiting per client IP
- Apply time-based restrictions (business hours, maintenance windows)
- Filter by HTTP method, User-Agent, Content-Type, or query parameters
- Combine multiple security rules with weight-based priority

### Key Features

- **Multi-layered security**: Combine 13 different rule types in a single policy
- **Performance optimized**: Pre-compiled IP networks and path routers
- **Hot-reload compatible**: Configuration changes apply without restart
- **Thread-safe rate limiting**: In-memory state tracking with RwLock
- **Flexible evaluation**: Weight-based priority with complete rule evaluation

## Evaluation Logic

**A request is ACCEPTED only if:**
1. It passes the IP allowlist gate, if any policy has an `ip_allow` rule (see [IP Allowlist Gate](#ip-allowlist-gate))
2. AND at least ONE allow rule matches (across all enabled policies/rules)
3. AND zero deny rules match

### Key Points

- **IP allowlists are a required gate**: A client that matches no `ip_allow` rule in any policy is denied, whatever other rules allow
- **All rules evaluated**: Not first-match-wins - every enabled rule is checked
- **Weight determines order**: Higher weight = evaluated first (but doesn't stop evaluation)
- **Implicit deny**: If no allow rules match, request is denied (403 Forbidden)
- **Deny overrides all**: A single matching deny rule blocks the request, regardless of allow rules

### Evaluation Flow

```
1. Filter to enabled policies only
2. IP allowlist gate. If any policy has enabled ip_allow rules:
   - Client IP unknown or unparseable → DENY (403, fail closed)
   - Client IP matches no ip_allow rule in any policy → DENY (403)
3. For each policy:
   a. Filter to enabled rules only
   b. Sort rules by weight (descending)
   c. Evaluate ALL rules in order
   d. Track: has_allow, has_deny
4. Final decision:
   - If has_deny = true → DENY (403)
   - Else if has_allow = false → DENY (implicit, 403)
   - Else → ALLOW (continue to backend)
```

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

```toml
[middleware.my_policy]
type = "policies"

# Policy 1
[[middleware.my_policy.options.policies]]
id = "access_control"
name = "Access Control Policy"
enabled = true

# Rule 1
[[middleware.my_policy.options.policies.rules]]
id = "rule1"
name = "Allow Internal Networks"
type = "ip_allow"
weight = 100
enabled = true

[middleware.my_policy.options.policies.rules.options]
ip_addresses = ["10.0.0.0/8", "192.168.0.0/16"]

# Rule 2
[[middleware.my_policy.options.policies.rules]]
id = "rule2"
name = "Deny Blocklist"
type = "ip_deny"
weight = 90
enabled = true

[middleware.my_policy.options.policies.rules.options]
ip_addresses = ["203.0.113.0/24"]
```

### Configuration Fields

**Policy fields:**
- `id` (string, optional): Unique identifier
- `name` (string, optional): Human-readable name
- `enabled` (boolean, default: true): Whether policy is active
- `rules` (array, required): Array of rules within this policy

**Rule fields:**
- `id` (string, optional): Unique identifier
- `name` (string, optional): Human-readable name
- `type` (string, required): Rule type (see [Rule Types](#rule-types))
- `weight` (integer, default: 0): Priority for evaluation order
- `enabled` (boolean, default: true): Whether rule is active
- `options` (table, varies): Rule-specific configuration

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

Policy rule types that match on client address, path, location, headers, and rate. See [Policy Rules: Source and Access](policies-rules-source.md).

### Policy Rules: Request Attributes

Policy rule types that match on time, method, user agent, content type, and query parameters. See [Policy Rules: Request Attributes](policies-rules-request.md).

## Weight and Priority

Rules are evaluated in weight order (higher weight = evaluated first):

```
Weight Range    Purpose                     Example
-----------     ---------                   -------
-1000 to -100   Critical overrides          Emergency blocks
-99 to -1       High priority rules         Specific allows/denies
0               Default priority            Most rules
1 to 99         Lower priority              Catch-all rules
100 to 1000     Fallback rules              Default deny-all
```

**Important:** Weight only affects evaluation ORDER. All enabled rules are still evaluated - it's not first-match-wins.

## Policy Patterns and Troubleshooting

Common policy patterns, best practices, troubleshooting, and examples. See [Policy Patterns and Troubleshooting](policies-patterns.md).

## Performance

### Pre-Compilation Optimizations

- **IP rules**: CIDR networks compiled at config load → O(log n) lookup
- **Path rules**: matchit router compiled at config load → O(log n) lookup
- **Rate limiting**: In-memory HashMap with RwLock → O(1) lookup
  - **Automatic cleanup**: Background task runs every 60 seconds
  - Removes entries older than 5 minutes (300 seconds)
  - Prevents unbounded memory growth from unique IPs
  - Debug logging when entries are removed
- **Header/Geo**: Simple string matching → O(1) per header

### Typical Performance

- **Overhead**: < 1ms per request with moderate rule counts (< 50 rules)
- **Memory**: ~1KB per rule (pre-compiled state)
- **Scalability**: Tested up to 1000 concurrent requests without degradation

### Hot Reload Support

Policies middleware supports hot configuration reload:
- Changes to policies/rules applied without restart
- Pre-compiled caches updated atomically (Arc swap)
- Rate limit state preserved across reloads
- Zero downtime for configuration changes

## See Also

- [Configuration Guide](configuration-file.md) - Overall configuration structure
- [Middleware Overview](middleware.md) - Other middleware types
- [Path Filter Middleware](middleware.md#path-filter) - Simple path-based filtering
- [Hot Reload](../architecture/config-reload.md) - Configuration reload architecture
