---
title: HTTP Backends
description: Configuration for the HTTP, FHIR, DICOMweb, and HTTP/3 backends.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/backends.md
---

# HTTP Backends

Configuration for the HTTP, FHIR, DICOMweb, and HTTP/3 backends. This page is part of [Backends](backends.md).

## HTTP (Passthru)

A basic HTTP backend for connecting to HTTP/HTTPS targets.

**Service behavior**:
- Accepts a `RequestEnvelope` and converts it to an HTTP request
- If a `Command` is set on the envelope, the Command's `Operation` overrides the HTTP method (e.g., Query/Retrieve→GET, Store→POST, Update→PUT). See [commands.md](../architecture/commands.md).
- Sends request to configured target
- Converts HTTP response back to `ResponseEnvelope`
- Forwards request headers as described in [Request Headers Sent Upstream](backends.md#request-headers-sent-upstream)
- Preserves response headers, status codes, and body

**Configuration**:
```toml
[backends.<name>]
service = "http"
[backends.<name>.options]
base_url = "https://api.example.com"
``` path=null start=null

**Example**: HTTP API backend
```toml
[backends.external_api]
service = "http"
[backends.external_api.options]
base_url = "https://external-api.example.com/v1"
``` path=null start=null

## FHIR

Extends the HTTP backend for FHIR resource servers.

**Service behavior**:
- FHIR-aware request/response handling
- Supports FHIR search parameters and operations
- Validates FHIR content types

**Configuration**:
```toml
[backends.<name>]
service = "fhir"
[backends.<name>.options]
base_url = "https://fhir.example.com/r4"
``` path=null start=null

**Example**: FHIR R4 server
```toml
[backends.fhir_server]
service = "fhir"
[backends.fhir_server.options]
base_url = "https://hapi.fhir.org/baseR4"
``` path=null start=null

## DICOMweb

Extends the HTTP backend for DICOMweb QIDO-RS/WADO-RS/STOW-RS targets.

**Service behavior**:
- DICOMweb-compliant request formatting
- Multipart handling for STOW-RS
- QIDO-RS query parameter construction

**Configuration**:
```toml
[backends.<name>]
service = "dicomweb"
[backends.<name>.options]
base_url = "https://dicomweb.example.com"
``` path=null start=null

**Example**: DICOMweb PACS
```toml
[backends.pacs_dicomweb]
service = "dicomweb"
[backends.pacs_dicomweb.options]
base_url = "https://pacs.example.com/dicomweb"
``` path=null start=null

## HTTP/3 (QUIC)

An HTTP/3 backend for connecting to targets over QUIC with TLS 1.3.

**Service behavior**:
- Connects to backends using HTTP/3 over QUIC (UDP transport)
- Built-in TLS 1.3 encryption (always enabled)
- Multiplexed streams without head-of-line blocking
- Supports custom CA certificates for self-signed servers
- If a `Command` is set on the envelope, the Command's `Operation` overrides the HTTP method, same as the HTTP backend. See [commands.md](../architecture/commands.md).

**Configuration**:
```toml
[backends.<name>]
service = "http3"
[backends.<name>.options]
host = "api.example.com"
port = 443
base_path = "/api/v1"
ca_cert_path = "/path/to/ca.pem"  # Optional: for self-signed certs
timeout_secs = 30                  # Optional: request timeout
``` path=null start=null

Alternatively, use the connection config format:
```toml
[backends.<name>]
service = "http3"
[backends.<name>.options.connection]
host = "api.example.com"
port = 443
protocol = "h3"
base_path = "/api/v1"
ca_cert_path = "/path/to/ca.pem"
``` path=null start=null

**Configuration Options**:
- `host` (string, required): Target hostname
- `port` (integer, optional): Target port (default: 443)
- `base_path` (string, optional): Base URL path prefix
- `ca_cert_path` (string, optional): Path to PEM-encoded CA certificate for validating self-signed or custom CA certificates
- `timeout_secs` (integer, optional): Request timeout in seconds (default: 30)

**Example**: HTTP/3 API backend
```toml
[backends.h3_api]
service = "http3"
[backends.h3_api.options]
host = "fast-api.example.com"
port = 443
base_path = "/v2"
``` path=null start=null

**Example**: HTTP/3 with custom CA
```toml
[backends.internal_h3]
service = "http3"
[backends.internal_h3.options]
host = "internal.corp.local"
port = 8443
ca_cert_path = "./certs/internal-ca.pem"
``` path=null start=null

**When to use HTTP/3**:
- High-latency or unreliable networks (mobile, satellite)
- Backend servers that support HTTP/3
- When you need connection migration (e.g., mobile clients changing networks)
- To avoid TCP head-of-line blocking on multiplexed connections
