# Data Meshes

A data mesh is a collection of Harmony proxies that can securely communicate with each other via mesh networking. This enables distributed API access patterns where proxies can route requests to other mesh members using JWT-based authentication.

## Overview

Mesh configuration consists of three components:

- **Mesh**: Groups ingress and egress definitions together with authentication settings
- **Ingress**: Defines how this proxy accepts requests from other mesh members (binds URLs to pipeline endpoints)
- **Egress**: Defines how this proxy sends requests to other mesh members (references pipeline backends)

Mesh networking enables scenarios like:
- Cross-team API access within an organization
- Partner integrations with secure authentication
- Distributed data pipelines spanning multiple gateways

## Configuration Structure

Harmony expects mesh configuration in a dedicated directory:

```toml
[proxy]
id = "my-gateway"
mesh_path = "mesh"  # default: "mesh" (relative to config.toml)
```

Directory structure:

```
config/
├── config.toml           # Main configuration with mesh_path = "mesh"
├── pipelines/            # Pipeline TOML files (define endpoints and backends)
│   └── *.toml
└── mesh/                 # Mesh TOML files (define mesh, ingress, egress)
    └── *.toml
```

Ingress and egress definitions are **nested within pipeline files** under `[pipelines.<name>.mesh]`, while mesh definitions group them in separate files under the `mesh/` directory.

## Mesh Definition

A mesh groups ingress and egress points together with authentication settings:

```toml
[mesh.my-mesh]
type = "http3"                                    # Protocol: "http" or "http3"
provider = "local"                                # Provider: "local" or "runbeam"
auth_type = "jwt"                                 # Auth type (default: "jwt")
jwt_secret = "your-secure-shared-secret-key"    # For HS256 symmetric auth
ingress = ["api-ingress", "webhook-ingress"]    # Ingress definitions
egress = ["partner-egress"]                      # Egress definitions
description = "Production mesh for partners"
enabled = true
```

### Mesh Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | string | Yes | - | Protocol type: `http` or `http3` |
| `provider` | string | Yes | - | Mesh provider: `local` or `runbeam` |
| `auth_type` | string | No | `jwt` | Authentication type (currently only `jwt`) |
| `jwt_secret` | string | No* | - | HS256 shared secret for JWT signing/verification |
| `jwt_private_key_path` | string | No* | - | Path to RSA private key (PEM format) for RS256 signing |
| `jwt_public_key_path` | string | No* | - | Path to RSA public key (PEM format) for RS256 verification |
| `jwks_url` | string | No | - | JWKS endpoint URL (Runbeam provider only) |
| `ingress` | array | Yes | - | List of ingress definition names (min: 1) |
| `egress` | array | Yes | - | List of egress definition names (min: 1) |
| `description` | string | No | - | Human-readable description |
| `enabled` | boolean | No | true | Whether the mesh is active |

*For `local` provider, you must configure either `jwt_secret` (HS256) or both RSA key paths (RS256). For `runbeam` provider, JWT handling is managed by Runbeam Cloud.

### Mesh Providers

**Local Provider**:
- Self-managed mesh with local JWT authentication
- JWT tokens are generated and validated locally
- Requires `jwt_secret` (HS256) or RSA key paths (RS256)
- Suitable for on-premises or single-organization deployments

**Runbeam Provider**:
- Runbeam Cloud managed mesh
- JWT tokens are fetched from and validated by Runbeam Cloud API
- No local JWT keys needed
- Suitable for cross-organization or cloud-hosted deployments

## Ingress Configuration

An ingress binds URLs to a pipeline's endpoint, optionally with mesh authentication.

**Location**: Nested within pipeline configuration files at `[pipelines.<name>.mesh.ingress.<name>]`

```toml
# In pipelines/api.toml
[pipelines.api.mesh.ingress.api-ingress]
type = "http"                                  # Protocol type
urls = ["https://api.example.com", "https://api2.example.com"]
endpoint = "api-endpoint"                      # Optional: defaults to first endpoint
mode = "default"                               # "default" or "mesh"
description = "API ingress for partner requests"
enabled = true
```

### Ingress Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | string | Yes | - | Protocol type: `http` or `http3` |
| `urls` | array | Yes | - | URLs that map to this ingress (min: 1) |
| `endpoint` | string | No | First endpoint | Optional endpoint override |
| `mode` | string | No | `default` | Request mode: `default` or `mesh` |
| `description` | string | No | - | Human-readable description |
| `enabled` | boolean | No | true | Whether the ingress is active |

### Ingress Mode

The `mode` field controls whether non-mesh requests are allowed:

- **`default`** (or omitted): All requests are processed regardless of mesh membership. If the ingress is in a mesh and the request has a valid mesh JWT, it's processed with mesh context. Otherwise, it proceeds without mesh context.

- **`mesh`**: Only requests with a valid mesh JWT matching this ingress's membership are allowed. Requests without a valid JWT are rejected with 403 Forbidden.

```toml
# Mesh-only ingress - rejects non-mesh requests
[pipelines.api.mesh.ingress.internal-api]
type = "http"
urls = ["https://internal.example.com"]
mode = "mesh"  # Only allows mesh requests
```

### Ingress Without Mesh

Ingresses work independently of meshes for simple URL→pipeline binding:

```toml
# Public API - no mesh required
[pipelines.my-pipeline.mesh.ingress.public-api]
type = "http"
urls = ["https://api.example.com/v1"]
# No mesh membership needed - requests routed without JWT auth
```

## Egress Configuration

An egress defines how this proxy sends requests to other mesh members.

**Location**: Nested within pipeline configuration files at `[pipelines.<name>.mesh.egress.<name>]`

```toml
# In pipelines/api.toml
[pipelines.api.mesh.egress.partner-egress]
type = "http3"                    # Protocol type
backend = "partner-backend"       # Optional: defaults to first backend
mode = "default"                  # "default" or "mesh"
description = "Egress to partner system"
enabled = true
```

### Egress Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | string | Yes | - | Protocol type: `http` or `http3` |
| `backend` | string | No | First backend | Optional backend override |
| `mode` | string | No | `default` | Request mode: `default` or `mesh` |
| `description` | string | No | - | Human-readable description |
| `enabled` | boolean | No | true | Whether the egress is active |

### Egress Mode

Similar to ingress, the `mode` field controls whether non-mesh requests are allowed:

- **`default`** (or omitted): All requests are processed regardless of mesh context. Requests with mesh context include JWT authentication.

- **`mesh`**: Only requests with mesh context are allowed. If the request didn't enter via a mesh ingress, the request is rejected with 403 Forbidden.

```toml
# Mesh-only egress - only allows requests with mesh context
[pipelines.api.mesh.egress.secure-partner]
type = "http3"
backend = "partner-backend"
mode = "mesh"  # Only allows requests with mesh context
```

## JWT Authentication

Mesh members authenticate with each other using JWT (JSON Web Tokens).

### How It Works

- **Egress (outgoing requests)**: JWT is automatically generated and attached to the `Authorization` header
- **Ingress (incoming requests)**: JWT in `Authorization` header is validated before processing

### JWT Claims

| Claim | Description |
|-------|-------------|
| `iss` | Issuer - the mesh name |
| `sub` | Subject - source proxy identifier |
| `aud` | Audience - target mesh member (optional) |
| `iat` | Issued at timestamp |
| `exp` | Expiration timestamp (typically 5 minutes) |
| `mesh_id` | Mesh identifier for validation |

### HS256 (Symmetric Key)

Use a shared secret for simple deployments where all mesh members share the same key:

```toml
[mesh.my-mesh]
type = "http"
provider = "local"
jwt_secret = "your-secure-shared-secret-at-least-32-characters-recommended"
ingress = ["api-ingress"]
egress = ["partner-egress"]
```

Generate a secure secret:
```bash
# Generate random 32-byte base64-encoded secret
openssl rand -base64 32
```

### RS256 (Asymmetric Keys)

Use RSA key pairs for more secure deployments where signing and verification use different keys:

```toml
[mesh.my-mesh]
type = "http"
provider = "local"
jwt_private_key_path = "/etc/harmony/mesh/private.pem"  # For signing (egress)
jwt_public_key_path = "/etc/harmony/mesh/public.pem"    # For verification (ingress)
ingress = ["api-ingress"]
egress = ["partner-egress"]
```

Generate RSA keys:
```bash
# Generate private key
openssl genrsa -out private.pem 2048

# Extract public key
openssl rsa -in private.pem -pubout -out public.pem
```

### Runbeam Provider

When using the Runbeam provider, JWT handling is managed by Runbeam Cloud:

```toml
[mesh.my-mesh]
type = "http3"
provider = "runbeam"
# No jwt_secret or key paths needed - managed by Runbeam Cloud
ingress = ["api-ingress"]
egress = ["partner-egress"]
```

## Complete Example

### Pipeline Configuration (`pipelines/healthcare.toml`)

```toml
[pipelines.healthcare]
description = "Healthcare data pipeline"
networks = ["default"]
endpoints = ["fhir-endpoint", "dicomweb-endpoint"]
backends = ["partner-fhir-backend", "partner-dicomweb-backend"]

# Ingress definitions (bind URLs to this pipeline)
[pipelines.healthcare.mesh.ingress.fhir-ingress]
type = "http"
urls = ["https://fhir.myorg.com/r4"]
endpoint = "fhir-endpoint"
description = "FHIR R4 API ingress"

[pipelines.healthcare.mesh.ingress.dicom-ingress]
type = "http3"
urls = ["https://dicom.myorg.com/wado-rs"]
endpoint = "dicomweb-endpoint"
description = "DICOMweb ingress"

# Egress definitions (for outgoing mesh requests)
[pipelines.healthcare.mesh.egress.partner-fhir]
type = "http3"
backend = "partner-fhir-backend"
description = "Egress to partner FHIR server"

[pipelines.healthcare.mesh.egress.partner-dicom]
type = "http3"
backend = "partner-dicomweb-backend"
description = "Egress to partner DICOMweb server"
```

### Mesh Configuration (`mesh/production.toml`)

```toml
# Mesh definition groups ingresses/egresses for JWT authentication
[mesh.production]
type = "http3"
provider = "local"
auth_type = "jwt"
jwt_secret = "production-mesh-secret-key-minimum-32-characters"
ingress = ["fhir-ingress", "dicom-ingress"]
egress = ["partner-fhir", "partner-dicom"]
description = "Production data mesh for healthcare integrations"
enabled = true
```

## Validation

Harmony validates mesh configuration at startup:

- Ingress must have at least one URL
- If endpoint is specified in ingress, it must exist in the referenced pipeline
- If backend is specified in egress, it must exist in the referenced pipeline
- Mesh must reference valid ingress and egress definitions
- URLs must be properly formatted
- Mesh names must match pattern `^[a-z0-9_-]+$`

Invalid configurations will cause startup to fail with descriptive error messages.

## Reference Syntax

For advanced configurations, meshes can include resources from remote providers using [resource references](./resource-references.md):

```toml
[mesh.cross-org-mesh]
type = "http3"
provider = "runbeam"
auth_type = "jwt"
ingress = [
    "local.name.local-api-ingress",                    # Local resource
    "runbeam.partner-team.ingress.name.their-api"      # Remote resource
]
egress = [
    "runbeam.partner-team.egress.name.their-backend"   # Remote resource
]
```

See [Resource References](./resource-references.md) for complete documentation on reference syntax and provider-based lookups.

## Hot-Reload Behavior

Mesh configuration changes are detected and applied automatically:

- **Mesh definition changes** (provider, auth, ingress/egress lists): Adapter restart required (~1-2s interruption)
- **Ingress/egress endpoints** (type, URLs, backend override): Zero-downtime update
- **JWT secrets**: Picked up on next request (zero-downtime)

See [Hot Configuration Reload](./config-reload.md) for detailed reload behavior.

## See Also

- [Configuration Schemas](./schema.md) - Technical reference for mesh schema
- [Providers](./providers.md) - Provider configuration for remote meshes
- [Resource References](./resource-references.md) - Reference syntax for cross-provider resources
- [Configuration](./configuration.md) - Main configuration documentation
