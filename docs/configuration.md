# Configuration

**Last Updated**: 2025-01-18 (Phase 6)

## Overview

Harmony uses a two-layer configuration model:
- **Top-level config**: Networks, storage, logging, service registrations
- **Pipeline files**: Endpoints, middleware, backends, and routing rules

**Protocol adapters** (HTTP, DIMSE, etc.) are automatically spawned based on pipeline configurations. See [adapters.md](adapters.md) for details.

Top-level config (examples/default/config.toml)
- [proxy]: service identity, logging level, and store_dir
- [network.<name>]: network interfaces and options
  - [network.<name>.http]: bind_address and bind_port
- pipelines_path: directory containing pipeline files
- transforms_path: directory for custom transforms (if used)
- [logging]: file logging options
- [services.*]: built-in or custom service types
- [middleware_types.*]: built-in or custom middleware types

Pipeline files (examples/default/pipelines/*.toml)
- `[pipelines.<name>]`: binds a set of endpoints, middleware, and backends to one or more networks
  - `networks`: list of network names from the top-level config
  - `endpoints`: list of endpoint names defined in this file
  - `middleware`: ordered list of middleware names (applied in sequence)
  - `backends`: list of backend names defined in this file
- `[middleware.<name>]`: middleware instances and their config
- `[endpoints.<name>]`: endpoint instances with service type and options
- `[backends.<name>]`: backend instances with service type and target configuration
- `[targets.<name>]`: concrete destinations that a backend selects from
- `[endpoint_types.*]`, `[service_types.*]`: register built-in or custom types

**Protocol adapters** are spawned automatically:
- **HttpAdapter**: Started for pipelines with HTTP/FHIR/JMIX/DICOMweb endpoints
- **DimseAdapter**: Started for pipelines with DICOM DIMSE endpoints
- See `src/lib.rs::run()` for orchestration logic

Validation expectations
- Networks must define valid HTTP bind_address and non-zero bind_port
- Each pipeline should reference at least one network, endpoint, and backend
- Unknown middleware names cause validation failure
- Middleware config is parsed by the middleware modules themselves

Examples
- Minimal passthrough: examples/default/pipelines/default.toml
- FHIR passthrough: examples/default/pipelines/fhir.toml
- FHIR to DICOM flow: examples/default/pipelines/fhir-dicom.toml

Notes
- Prefer ./tmp for temporary files rather than /tmp
- For realistic JWT auth configuration, see docs/middleware.md
