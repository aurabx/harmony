---
title: Configuration
description: Reference for the main configuration file, including networks, authentications, reload, and Runbeam Cloud.
status: active
updated: 2026-10-25
type: Configuration
---

# Configuration

## Environment Variable Substitution

How to reference environment variables inside configuration files. See [Environment Variable Substitution](environment-substitution.md).

## Overview

Harmony uses a two-layer configuration model:
- **Top-level config**: Networks, providers, logging, service registrations
- **Pipeline files**: Endpoints, middleware, backends, routing rules, and ingress/egress definitions
- **Mesh files**: Mesh definitions that group ingress/egress for inter-proxy communication

**Protocol adapters** (HTTP, DIMSE, etc.) are automatically spawned based on pipeline configurations. See [adapters.md](../architecture/adapters.md) for details.

### Configuration Schemas

All configuration is validated against formal schemas defined in `harmony-dsl`. For detailed technical reference, see:

- **[Configuration Schemas](schema.md)** - Overview of all harmony configuration schemas
- **[Mesh Configuration](mesh.md)** - Data mesh networking and ingress/egress definitions
- **[Providers](providers.md)** - Provider configuration for resource resolution and cloud polling
- **[Resource References](resource-references.md)** - Reference syntax for cross-provider resources

### Top-Level Configuration Structure

Top-level config (typically `config.toml`):
- `[proxy]`: Service identity, paths (pipelines, transforms, mesh), limits, and cloud polling
- `[provider.*]`: Provider configurations for resource resolution and polling
- `[management]`: Management API settings
- `[network.<name>]`: Network interface configurations
  - `[network.<name>.http]`: HTTP/1.x and HTTP/2 listener settings
  - `[network.<name>.http3]`: HTTP/3 (QUIC) listener settings
- `[logging]`: File logging and log level configuration
- `[authentications.<name>]`: Credentials that targets, backends and middleware refer to by name. See [Authentications](#authentications).
- `[runbeam]`: [DEPRECATED] Legacy Runbeam Cloud settings (use `[provider.runbeam]` instead)

### Pipeline Configuration Structure

Pipeline files (typically in `pipelines/` directory):
- `[pipelines.<name>]`: Pipeline definition grouping endpoints, middleware, backends
  - `networks`: List of network names to bind this pipeline to
  - `endpoints`: Endpoints this pipeline handles
  - `middleware`: Ordered middleware chain
  - `backends`: Backend targets
- `[pipelines.<name>.mesh.ingress.*]`: Ingress definitions (URLs → endpoints)
- `[pipelines.<name>.mesh.egress.*]`: Egress definitions (backends → mesh targets)
- `[middleware.<name>]`: Middleware configuration
- `[endpoints.<name>]`: Endpoint configuration
- `[backends.<name>]`: Backend configuration
- `[targets.<name>]`: Concrete destinations for backends

### Mesh Configuration Structure

Mesh files (typically in `mesh/` directory):
- `[mesh.<name>]`: Mesh definition grouping ingress/egress with authentication

Ingress and egress are **nested within pipeline files**, while mesh definitions **group them** in separate mesh files. See [Mesh Configuration](mesh.md) for complete documentation.

### Protocol Adapters

Adapters are started automatically based on configuration:
- **HttpAdapter**: For HTTP/FHIR/JMIX/DICOMweb endpoints (TCP: HTTP/1.x, HTTP/2, optional TLS)
- **Http3Adapter**: For networks with HTTP/3 configuration (QUIC-based, requires TLS)
- **DimseAdapter**: For DICOM DIMSE endpoints
- See `src/lib.rs::run()` for orchestration logic

### Validation Expectations

- Networks must define valid HTTP bind_address and non-zero bind_port
- HTTP/3 networks must specify valid cert_path and key_path for TLS
- HTTPS networks must specify both cert_path and key_path
- Each pipeline should reference at least one network, endpoint, and backend
- Mesh definitions must reference valid ingress/egress definitions
- If ingress/egress specify endpoint/backend overrides, those must exist in the pipeline
- All referenced providers must be configured
- Unknown middleware names cause validation failure
- Every `[middleware.<name>]` instance must build from its options. Harmony resolves each instance at load and reload, the same way it does per request. A bad instance fails validation with `InvalidMiddleware`. Examples: a `policies` entry that is not a policy ID, an unknown policy or rule ID, a missing `spec_path` file, or an unreadable JWT public key.
- Every `authentication` reference must name a key in `[authentications]`. See [Authentications](#authentications).

### Examples

- Minimal passthrough: examples/default/pipelines/default.toml
- FHIR passthrough: examples/default/pipelines/fhir.toml
- FHIR to DICOM flow: examples/default/pipelines/fhir-dicom.toml
- Data mesh: examples/default/pipelines/data-mesh/ (includes mesh/, pipelines/, config.toml)

## Authentications

Define each credential once under `[authentications.<key>]`. Targets, backends, peers, endpoints and middleware instances then refer to it by key.

```toml
[authentications.fhir_bearer]
id = "fhir_bearer"
method = "bearer"            # bearer, basic, api_key, none or jwt
[authentications.fhir_bearer.options]
token = "$FHIR_TOKEN"

[targets.fhir_server]
connection.host = "fhir.example.com"
connection.protocol = "https"
authentication = "fhir_bearer"
```

A backend with `target_ref` inherits the target's `authentication`. A backend can set its own `authentication` to override it.

Peers and endpoints accept an `authentication` reference, and Harmony checks it at load. Harmony does not use it after that, so it has no effect on requests yet.

### Methods and required options

| Method | Required options | Sends |
|---|---|---|
| `bearer` | `token` | `Authorization: Bearer <token>` |
| `basic` | `username`, `password` | `Authorization: Basic <base64 of username:password>` |
| `api_key` | `api_key`. Optional `header_name`, default `X-API-Key` | `<header_name>: <api_key>` |
| `none` | none | nothing |
| `jwt` | checked by the jwt_auth middleware | nothing. Settings for jwt_auth middleware only |

A required option must be a non-empty string. `header_name` must be a valid HTTP header name.

Targets, backends and webhook middleware send the credential as a request header, so they cannot reference a `jwt` definition. Other middleware may reference any method.

### Invalid definitions fail the load

Harmony checks every definition in `[authentications]` when it loads the config, including definitions that nothing references. The load fails if a definition has an unknown method or is missing a required option. It also fails if a target, backend or webhook references a `jwt` definition:

```
ERROR: Configuration reference resolution failed: Authentication 'fhir_bearer' is invalid: method 'bearer' requires a non-empty 'token' option
```

Startup, `--validate-config` and hot reload handle this error the same way as an unresolved reference.

Credentials are set by reference only. A backend, endpoint or middleware options table that contains `authentication_def` fails the load. Define the credential under `[authentications]` and set `authentication = "<key>"` instead.

### Reference format

An `authentication` value is the bare key, like `target_ref` and `peer_ref`. Write `authentication = "fhir_bearer"`. Do not write `authentication = "authentications.fhir_bearer"`. The prefixed form is rejected.

### Unresolved references fail the load

Harmony checks every `authentication` reference when it loads the config. If a reference does not match a key in `[authentications]`, the load fails. The error names the item, the missing key and the keys that exist:

```
ERROR: Configuration reference resolution failed: Target 'fhir_server' references unknown authentication 'authentications.fhir_bearer'. Drop the "authentications." prefix: use "fhir_bearer" instead of "authentications.fhir_bearer". Available authentications: fhir_bearer
```

- At startup, and with `--validate-config`, Harmony prints the error on stderr and exits with code 1.
- On hot reload, Harmony logs `Config reload failed` and keeps the previous config.

Harmony reports every bad reference in one error, one per line.

### Definitions in other files

`[authentications]` can be in the top-level config file or in any pipeline, transform or mesh file. Harmony merges them all. Each key must be defined once. If two files define the same key, the load fails and the error names both files.

## Network Configuration

How to configure networks, bind addresses, TLS, and WireGuard. See [Network Configuration](networks.md).

## Hot Configuration Reload

Which configuration changes reload without a restart, and how to use reload. See [Hot Configuration Reload](hot-reload.md).

## Runbeam Cloud Integration

How to connect Harmony to Runbeam Cloud for authorisation and configuration polling. See [Runbeam Cloud Integration](../integrations/runbeam-cloud.md).

## Environment Variables

Environment variables that Harmony reads at startup. See [Environment Variables](environment.md).
