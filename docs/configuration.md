# Configuration

Harmony uses a two-layer configuration model:
- A top-level config file (e.g., examples/default/config.toml)
- One or more pipeline files (e.g., examples/default/pipelines/*.toml)

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
- [pipelines.<name>]: binds a set of endpoints, middleware, and backends to one or more networks
  - networks: list of network names from the top-level config
  - endpoints: list of endpoint names defined in this file
  - middleware: list of middleware names defined in this file
  - backends: list of backend names defined in this file
- [middleware.<name>]: middleware instances and their config
- [endpoints.<name>]: endpoint instances and their config
- [backends.<name>]: backend instances and their config
- [targets.<name>]: concrete destinations that a backend selects from
- [endpoint_types.*], [service_types.*]: register built-in or custom types

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
