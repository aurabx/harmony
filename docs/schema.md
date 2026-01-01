# Configuration Schemas

Harmony configuration is validated against a set of schemas defined in the harmony-dsl project. These schemas enforce structure, types, enums, and validation rules across all configuration files.

## Overview

Configuration validation happens when Harmony loads TOML files. The schemas define:

- **Structure**: Which sections and fields are valid
- **Types**: Integer, string, boolean, array, table
- **Constraints**: Required fields, min/max values, pattern matching, enums
- **Defaults**: Default values when fields are omitted
- **Relationships**: How fields in one section relate to others

Schemas are defined in `harmony-dsl` and exported for use by:
- **harmony-proxy** (Rust): Runtime validation during startup and hot-reload
- **runbeam** (Laravel): Backend validation when receiving configuration from cloud API
- **runbeam-sdk** (Rust SDK): Shared type definitions

## Schema Files

### Core Configuration Schemas

#### harmony-config-schema.toml (v1.11.0)

Defines the main gateway configuration file structure.

**Location**: `projects/harmony-dsl/harmony-config-schema.toml`

**Main Tables**:

| Table | Purpose | Pattern | Required |
|-------|---------|---------|----------|
| `proxy` | Core proxy settings (id, paths, limits) | Fixed | Yes |
| `provider.*` | Provider configurations for resource resolution | Pattern | No |
| `management` | Management API settings | Fixed | No |
| `network.*` | Network interface configurations | Pattern | No |
| `network.*.http` | HTTP/1.x and HTTP/2 listener settings | Nested | No |
| `network.*.http3` | HTTP/3 (QUIC) listener settings | Nested | No |
| `logging` | File logging and log level configuration | Fixed | No |
| `runbeam` | [DEPRECATED] Legacy Runbeam Cloud settings | Fixed | No |

**Proxy Table Fields**:

- `id` (string, required): Unique proxy identifier
- `pipelines_path` (string, default: "pipelines"): Directory for pipeline configs
- `transforms_path` (string, default: "transforms"): Directory for transform specs
- `mesh_path` (string, default: "mesh"): Directory for mesh configurations
- `jwks_cache_duration_hours` (integer, default: 24, range: 1-168): JWKS cache duration
- `required_env_vars` (array of strings): Environment variables required at startup
- `sensitive_field_patterns` (array of strings): Field patterns to mask in logs
- `primary_provider` (string, default: "runbeam"): Primary provider for cloud polling

**Provider Pattern Fields**:

Provider names must match `^[a-z0-9_-]+$`.

- `api` (string, required for remote providers): Base URL for provider API
- `poll_interval_secs` (integer, default: 30, range: 0-3600): Polling interval in seconds

Special case: The `local` provider is implicit and doesn't require `[provider.local]` section.

**Content Limits Table**:

All fields under `[proxy.content_limits]`:

- `max_body_size` (integer, default: 10485760, min: 1024): Max request body in bytes
- `max_csv_rows` (integer, default: 10000, min: 1): Max CSV rows to parse
- `max_xml_depth` (integer, default: 100, range: 1-1000): Max XML nesting depth
- `max_multipart_files` (integer, default: 10, min: 1): Max files per multipart request
- `max_form_fields` (integer, default: 1000, min: 1): Max form fields per request

#### harmony-pipeline-schema.toml (v1.11.0)

Defines pipeline configuration file structures (endpoints, backends, middleware).

**Location**: `projects/harmony-dsl/harmony-pipeline-schema.toml`

**Main Tables**:

| Table | Purpose | Pattern | Required |
|-------|---------|---------|----------|
| `pipelines.*` | Pipeline definitions | Pattern | No |
| `pipelines.*.mesh.ingress.*` | Ingress definitions within pipelines | Nested Pattern | No |
| `pipelines.*.mesh.egress.*` | Egress definitions within pipelines | Nested Pattern | No |
| `endpoints.*` | Endpoint definitions | Pattern | No |
| `backends.*` | Backend definitions | Pattern | No |
| `middleware.*` | Middleware configurations | Pattern | No |
| `services.*` | Service type registrations | Pattern | No |

**Pipeline Mesh Ingress Fields**:

Nested at `[pipelines.<name>.mesh.ingress.<name>]`:

- `type` (string, required, enum: "http", "http3"): Protocol type
- `mode` (string, default: "default", enum: "default", "mesh"): Request mode
- `endpoint` (string, optional): Optional endpoint override (defaults to first endpoint)
- `urls` (array of strings, required, min: 1): URLs that map to this ingress
- `description` (string, optional): Human-readable description
- `enabled` (boolean, default: true): Whether ingress is active

**Pipeline Mesh Egress Fields**:

Nested at `[pipelines.<name>.mesh.egress.<name>]`:

- `type` (string, required, enum: "http", "http3"): Protocol type
- `mode` (string, default: "default", enum: "default", "mesh"): Request mode
- `backend` (string, optional): Optional backend override (defaults to first backend)
- `description` (string, optional): Human-readable description
- `enabled` (boolean, default: true): Whether egress is active

#### harmony-mesh-schema.toml (v1.11.0)

Defines mesh configuration file structures for inter-proxy communication.

**Location**: `projects/harmony-dsl/harmony-mesh-schema.toml`

**Main Tables**:

| Table | Purpose | Pattern | Required |
|-------|---------|---------|----------|
| `mesh.*` | Mesh definitions | Pattern | No |

**Mesh Table Fields**:

Mesh names must match `^[a-z0-9_-]+$`.

- `id` (string, optional): Unique identifier for this mesh
- `type` (string, required, enum: "http", "http3"): Protocol type for mesh communication
- `provider` (string, required, enum: "local", "runbeam"): Mesh provider
- `auth_type` (string, default: "jwt", enum: "jwt"): Authentication type
- `jwt_secret` (string, optional): HS256 shared secret for JWT signing/verification
- `jwt_private_key_path` (string, optional): Path to RSA private key (PEM format)
- `jwt_public_key_path` (string, optional): Path to RSA public key (PEM format)
- `jwks_url` (string, optional): JWKS endpoint URL for Runbeam provider
- `ingress` (array of strings, required, min: 1): Ingress references
- `egress` (array of strings, required, min: 1): Egress references
- `description` (string, optional): Human-readable description
- `enabled` (boolean, default: true): Whether mesh is active

**Ingress Reference Syntax**:

Ingress array supports multiple formats:
- Bare name: `my_ingress` → resolves to local ingress
- Explicit local: `local.name.my_ingress` → explicit local lookup
- Provider ID: `runbeam.id.abc123` → provider-wide ID lookup
- Full path: `runbeam.partner.ingress.name.api` → full provider reference

#### harmony-remote-ingress-schema.toml (v1.10.0)

Defines remote ingress catalogue entries for mesh networking.

**Location**: `projects/harmony-dsl/harmony-remote-ingress-schema.toml`

**Main Tables**:

| Table | Purpose | Pattern | Required |
|-------|---------|---------|----------|
| `remote_ingress.*` | Remote ingress entries | Pattern | No |

**Remote Ingress Fields**:

- `urls` (array of strings, required, min: 1): List of URLs served by remote ingress

## Schema Validation Flow

When Harmony loads configuration:

1. **Parse TOML**: Load and parse TOML files
2. **Validate Structure**: Check against schema constraints
   - All required fields present
   - Field types match schema
   - Enum values are valid
   - Numeric constraints (min, max) are met
   - Pattern constraints (`^[a-z0-9_-]+$`) match
3. **Validate References**: Verify cross-references are valid
   - Ingress references valid endpoints (if specified)
   - Egress references valid backends (if specified)
   - Mesh ingress/egress arrays reference valid definitions
4. **Resolve Paths**: Expand relative paths (pipelines_path, mesh_path, etc.)
5. **Hot-Reload**: On file change, repeat validation before applying

Failed validation causes:
- **Startup**: Application fails to start with error details
- **Hot-Reload**: Configuration reverts to previous state, error logged

## Schema Versioning

Each schema has a version field (e.g., `version = "1.11.0"`):

- **Major**: Breaking changes (e.g., required field added, enum removed)
- **Minor**: Backward-compatible additions (new optional fields)
- **Patch**: Documentation or constraint refinements

Current versions:
- harmony-config-schema: 1.11.0
- harmony-pipeline-schema: 1.11.0
- harmony-mesh-schema: 1.11.0
- harmony-remote-ingress-schema: 1.10.0

## Field Constraints by Type

### String Fields

- `required`: Must be present and non-empty
- `optional`: Can be omitted or empty
- `enum`: Must be one of specified values (e.g., `["http", "http3"]`)
- `pattern`: Must match regex (e.g., `^[a-z0-9_-]+$`)

### Integer Fields

- `required`: Must be present and valid integer
- `optional`: Can be omitted (uses default if specified)
- `min`/`max`: Value must be within range (inclusive)

### Boolean Fields

- `required`: Must be present
- `optional`: Can be omitted (uses default if specified)
- `default`: Value used if omitted

### Array Fields

- `array_item_type`: Type of array elements (string, integer, etc.)
- `required`: Array must be present and non-empty (use `min_items`)
- `min_items`: Minimum number of items required
- `max_items`: Maximum number of items allowed

### Table Fields

- `pattern`: Field name pattern for dynamic tables (e.g., `provider.*`)
- `pattern_constraint`: Regex for field names in pattern tables
- `required_if`: Conditional requirement (e.g., `enabled == true`)

## Related Documentation

- [Mesh Configuration](./mesh.md) - Comprehensive guide to mesh configuration
- [Providers](./providers.md) - Provider configuration and reference resolution
- [Resource References](./resource-references.md) - Reference syntax guide
- [Configuration](./configuration.md) - Main configuration documentation

## See Also

- Schema source files: `projects/harmony-dsl/harmony-*-schema.toml`
- Harmony-DSL README: `projects/harmony-dsl/README.md`
- Config loading: `projects/harmony-proxy/src/config/config.rs`
