---
title: Utility Middleware
description: Reference for the log dump, webhook, and passthru middleware.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/middleware.md
---

# Utility Middleware

Reference for the log dump, webhook, and passthru middleware. This page is part of [Middleware](middleware.md).

## Log Dump

The log dump middleware outputs request or response envelopes to the logs to help builders and tools create and debug pipelines. It provides a comprehensive view of the current request/response state, including original and normalized data, making it particularly useful when placed after transformations.

### Configuration

Config keys:
- `apply` (string, optional): When to dump - "left", "right", or "both" (default: "both")
- `pretty` (bool, optional): Pretty print JSON output (default: true)
- `max_bytes` (integer, optional): Maximum bytes to include in logs for large content (default: 65536)
- `redact_headers` (array of strings, optional): Header names to redact (case-insensitive)
- `redact_metadata` (array of strings, optional): Metadata keys to redact
- `redact_data_fields` (array of strings, optional): Normalized data fields to redact (dot path notation)
- `label` (string, optional): Optional label to distinguish multiple dump points

### Example

```toml
[middleware.debug_dump]
type = "log_dump"
[middleware.debug_dump.options]
apply = "both"  # Dump both request and response
pretty = true   # Pretty print JSON
redact_headers = ["authorization", "cookie"]  # Redact sensitive headers
redact_metadata = ["api_key", "token"]     # Redact sensitive metadata
redact_data_fields = ["ssn", "password", "user.payment_details"]  # Redact normalized data fields
label = "after_transform"  # Help identify where in pipeline this occurred
max_bytes = 32768  # Limit for very large payloads
```

### Behavior

**Output includes**:
- Request/response details (method, URI, headers, cookies, query params, metadata)
- Normalized data and pre-transform snapshots (if available)
- Target details (backend routing information)
- Content metadata (format, parsing status, size)
- Configuration label and side indicator (left/right)

**Security considerations**:
- Always use redact options when dealing with production logs to avoid leaking PII or credentials
- Consider setting appropriate `max_bytes` to avoid log flooding
- Use `label` to help identify specific pipeline stages when using multiple dump points

**Log targeting**:
- All dump output uses the `harmony.dump` logging target
- Use `RUST_LOG=harmony.dump=info` to enable just dump logs (or `debug` for more verbose)
- Standard log filtering applies (can be directed to different files/destinations)

## Webhook

Sends pipeline request/response data to an external HTTP endpoint. Useful for audit logging, notifications, or external processing.

Config keys:
- `endpoint` (string, required): URL to POST data to
- `apply` (string, optional): When to apply - "left", "right", or "both" (default: "left")
- `redact_headers` (array of strings, optional): Header names to redact
- `redact_metadata` (array of strings, optional): Metadata keys to redact
- `timeout_secs` (integer, optional): Request timeout in seconds (default: 5)
- `authentication_def` (object, optional): Authentication configuration for the webhook request

Example:
```toml
[middleware.audit_webhook]
type = "webhook"
[middleware.audit_webhook.options]
endpoint = "https://audit.example.com/logs"
apply = "both"
redact_headers = ["authorization", "cookie"]
timeout_secs = 2
```

## Passthru

A no-op middleware that passes requests and responses through unchanged. Useful for testing, placeholders, or temporarily disabling middleware logic without removing the configuration entry.

Config:
```toml
[middleware.noop]
type = "passthru"
```
