---
title: Protocol Adapters
description: The ProtocolAdapter trait, the built-in HTTP, HTTP/3 and DIMSE adapters, and how to add a new one.
status: active
updated: 2026-10-24
type: Architecture
---

# Protocol Adapters

## Overview

Protocol adapters are the foundation of Harmony's protocol-agnostic architecture. Each protocol (HTTP, DIMSE, HL7, etc.) has a dedicated adapter that handles protocol-specific I/O while using the unified `PipelineExecutor` for all business logic.

> **Related**: For creating custom services and middleware that work with protocol adapters, see [extensions.md](extensions.md).

**Dynamic Adapter Selection**: Harmony automatically determines which protocol adapters to start for each network based on the services configured in that network's pipelines. This means:
- Networks with only HTTP-based services (http, jmix, fhir, etc.) start only the `HttpAdapter`
- Networks with `[network.<name>.http3]` configuration also start the `Http3Adapter`
- Networks with only DICOM SCP endpoints start only the `DimseAdapter`
- Networks with mixed service types start multiple adapters as needed
- Networks with no recognized services log a warning but don't fail

## Architecture

``` path=null start=null
Protocol Request
  ↓
ProtocolAdapter
  ├─ Protocol-specific I/O (listening, parsing)
  ├─ Convert to ProtocolCtx + RequestEnvelope
  ├─ Call PipelineExecutor (unified logic)
  ├─ Convert ResponseEnvelope back
  └─ Protocol-specific response formatting
  ↓
Protocol Response
``` path=null start=null

**Key Principle**: Protocol adapters handle I/O; `PipelineExecutor` handles business logic.

## ProtocolAdapter Trait

All adapters implement the `ProtocolAdapter` trait:

```rust
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    /// Returns the protocol this adapter handles
    fn protocol(&self) -> Protocol;

    /// Start the adapter (listener, server, etc.)
    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>>;

    /// Returns a human-readable summary
    fn summary(&self) -> String;
}
``` path=null start=null

## Protocol to Service Mapping

Each service type declares which protocol adapter it requires through the `ServiceType::required_protocol()` method. This design ensures:

- **No hardcoded mappings**: The protocol requirement lives with the service definition
- **Extensibility**: New services automatically work by implementing the trait method
- **Type safety**: The compiler ensures every service declares its protocol

Current protocol assignments:

| Protocol | Service Types | Notes |
|----------|---------------|-------|
| **HTTP** | `http`, `jmix`, `fhir`, `dicomweb`, `echo`, `management`, `mock_dicom`, `dicom`, `dicom_scu`, `http3` | Default for most services |
| **DIMSE** | `dicom_scp` | DICOM network listener |

**Note**: The `http3` backend service connects to external targets using HTTP/3, but receives requests via the standard HTTP adapter. The `Http3Adapter` is for _incoming_ HTTP/3 connections to Harmony itself.

**Note**: The `dicom` and `dicom_scu` services are used for backends (outgoing DICOM requests). They require the HTTP adapter to receive incoming HTTP requests, then make outgoing DIMSE calls via the SCU client.

This mapping is automatically applied when starting networks based on each service's declared protocol requirement.

## Available Adapters

### HttpAdapter

**Location**: `src/adapters/http/`

**Protocol**: HTTP/HTTPS

**Features**:
- Axum-based web server with optional TLS/HTTPS support
- Route matching: the most specific endpoint wins when endpoint paths overlap. Same-shape routes fail config load. See [router.md](router.md#http-route-matching)
- Header and body mapping to/from envelopes
- Routes GET, HEAD, POST, PUT, PATCH, DELETE and OPTIONS to the pipeline for general-purpose endpoints (`http`, `http_streaming`, `echo`, `fhir`). Protocol endpoints (`jmix`, `dicomweb`, management API) register only the methods their protocol defines. TRACE and CONNECT are never routed.
- TLS 1.3 with HTTP/1.1 and HTTP/2 ALPN when TLS is enabled

**Configuration**:

**Plain HTTP** (default):
```toml
[network.default.http]
bind_address = "0.0.0.0"
bind_port = 8080
``` path=null start=null

**HTTPS with TLS**:
```toml
[network.secure.http]
bind_address = "0.0.0.0"
bind_port = 443
cert_path = "/etc/harmony/certs/fullchain.pem"
key_path = "/etc/harmony/certs/privkey.pem"
``` path=null start=null

When both `cert_path` and `key_path` are provided, the adapter automatically enables HTTPS. The certificate and private key must be in PEM format.

**Supported Key Formats**:
- PKCS#8 (preferred)
- RSA PKCS#1 (legacy)

**HTTP to HTTPS Redirect**:
Use `force_https = true` to redirect all HTTP requests to HTTPS:
```toml
[network.redirect.http]
bind_address = "0.0.0.0"
bind_port = 80
force_https = true
``` path=null start=null

This returns HTTP 301 redirects to the `https://` equivalent URL. Only works when TLS is not configured on the network (no cert/key paths).

**Usage**:
```rust
let adapter = HttpAdapter::new(network_name, bind_addr, tls_config);
let handle = adapter.start(config, shutdown).await?;
``` path=null start=null

### Http3Adapter

**Location**: `src/adapters/http3/`

**Protocol**: HTTP/3 over QUIC

**Features**:
- QUIC-based HTTP/3 server using pure-Rust stack (quinn + h3 + rustls)
- TLS 1.3 with ALPN negotiation
- Multiplexed streams without head-of-line blocking
- 0-RTT connection resumption support (future enhancement)
- Shares pipelines with HttpAdapter on the same network

**Usage**:
```rust
let adapter = Http3Adapter::from_network(network_name, &network_config)?;
let handle = adapter.start(config, shutdown).await?;
``` path=null start=null

**Configuration**:
The Http3Adapter starts when a network has `[network.<name>.http3]` configuration:

```toml
[network.default.http3]
bind_address = "0.0.0.0"
bind_port = 443
cert_path = "/etc/harmony/certs/fullchain.pem"
key_path = "/etc/harmony/certs/privkey.pem"
``` path=null start=null

**Requirements**:
- TLS certificate and private key in PEM format
- UDP port access (HTTP/3 uses QUIC over UDP)
- Client must support HTTP/3 (e.g., curl with `--http3` flag)

**Coexistence with HTTP/1.x**:
A network can expose both HTTP/1.x (via HttpAdapter) and HTTP/3 (via Http3Adapter) simultaneously on different ports. Both adapters serve the same pipelines and endpoints.

### DimseAdapter

**Location**: `src/adapters/dimse/`

**Protocol**: DICOM DIMSE

**Features**:
- DIMSE SCP (Service Class Provider) listener
- Support for C-ECHO, C-FIND, C-MOVE, C-GET, C-STORE
- AE title-based routing
- Dataset encoding/decoding
- Integration with PipelineExecutor via QueryProvider trait

**Service Integration**:
The DimseAdapter works with `dicom_scp` endpoints to receive incoming DICOM requests. It does not handle backend/SCU operations - those are handled by the `dicom_scu` service through the SCU client.

**Usage**:
```rust
let adapter = DimseAdapter::new(network_name);
let handle = adapter.start(config, shutdown).await?;
``` path=null start=null

**Configuration Detection**:
The adapter automatically detects and starts SCP listeners for endpoints with `service = "dicom_scp"`. Legacy `service = "dimse"` is also supported with a deprecation warning.

## Implementing a Protocol Adapter

How to implement a new protocol adapter, with best practices and common patterns. See [Implementing a Protocol Adapter](adapters-implementing.md).

## See Also

- [docs/architecture/router.md](router.md) - Pipeline architecture and flow
- [docs/architecture/diagrams.md](diagrams.md) - Architecture diagrams
- [src/adapters/http/](../../src/adapters/http) - HTTP adapter reference implementation
- [src/adapters/dimse/](../../src/adapters/dimse) - DIMSE adapter reference implementation
