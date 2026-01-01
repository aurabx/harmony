# Resource References

Resource references allow you to use resources from different providers in your Harmony configuration. This enables cross-gateway and cross-team resource sharing.

## Overview

A reference is a string that identifies a resource by its provider, type, team, and name. References are resolved at runtime, allowing you to connect to resources managed by external systems like Runbeam Cloud.

References are most commonly used in mesh configurations to connect ingresses and egresses across gateways.

## Reference Syntax

References support several formats depending on how you want to identify the resource:

```
<name>                                  # Bare name (shorthand for local.name.<name>)
local.name.<name>                       # Explicit local lookup
<provider>.id.<id>                      # Provider-wide ID lookup
<provider>.<team>.id.<id>               # Team-scoped ID lookup
<provider>.<team>.<type>.name.<name>    # Full path by name
<provider>.<team>.<type>.id.<id>        # Full path by ID
```

### Format Details

| Format | Description | Example |
|--------|-------------|---------|
| `<name>` | Bare name (shorthand for local lookup) | `api-ingress` |
| `local.name.<name>` | Explicit local resource by name | `local.name.api-ingress` |
| `<provider>.id.<id>` | Provider-scoped reference by ID | `runbeam.id.abc123def456` |
| `<provider>.<team>.id.<id>` | Team-scoped reference by ID | `runbeam.acme-corp.id.abc123` |
| `<provider>.<team>.<type>.name.<name>` | Full reference by name | `runbeam.acme-corp.ingress.name.fhir-api` |
| `<provider>.<team>.<type>.id.<id>` | Full reference by ID | `runbeam.acme-corp.ingress.id.abc123` |

### Components

| Component | Description | Example |
|-----------|-------------|---------|
| `<provider>` | Name of the configured provider | `runbeam`, `local` |
| `<team>` | Team/organization that owns the resource | `acme-corp`, `my-team` |
| `<type>` | Type of resource | `ingress`, `egress` |
| `id.<id>` | Resource identifier by unique ID | `id.abc123def456` |
| `name.<name>` | Resource identifier by name | `name.fhir-api` |

### Examples

```
my-ingress                                        # Bare name (local)
local.name.my-ingress                             # Explicit local
runbeam.id.abc123def456                           # Provider ID lookup
runbeam.acme-healthcare.ingress.name.fhir-api     # Full path by name
runbeam.partner-team.egress.name.dicom-store      # Full path by name
```

## Local References

For resources defined in your local configuration, use bare names or the explicit `local.name.<name>` format:

```toml
[mesh.my-mesh]
type = "http"
provider = "local"
ingress = [
    "api-ingress",                  # Bare name (recommended)
    "webhook-ingress",              # Bare name
    "local.name.partner-api"        # Explicit local format
]
egress = [
    "local.name.partner-egress"     # Explicit local
]
```

Local references are resolved from your local configuration:
- Pipeline file ingress definitions under `[pipelines.<name>.mesh.ingress.*]`
- Pipeline file egress definitions under `[pipelines.<name>.mesh.egress.*]`
- Remote ingress catalogue under `[remote_ingress.*]`

## Provider References

To reference resources from a remote provider, use the provider reference syntax:

```toml
[mesh.cross-org-mesh]
type = "http3"
provider = "runbeam"
ingress = [
    "local.name.local-ingress",                     # Local resource
    "runbeam.partner-team.ingress.name.their-api"  # Remote resource
]
egress = [
    "runbeam.partner-team.egress.name.their-backend"  # Remote resource
]
```

Provider references are resolved at runtime by:
1. Parsing the reference string
2. Looking up the provider configuration in `[provider.*]`
3. Calling the provider's API to retrieve the resource
4. Caching the result for subsequent requests

## Mesh Configuration Example

Here's a complete example using references:

```toml
# mesh/production.toml
[mesh.production]
type = "http3"
provider = "runbeam"
auth_type = "jwt"
ingress = [
    "api-ingress",                                      # Local
    "runbeam.partner.ingress.name.partner-ingress"     # From partner team
]
egress = [
    "backend-egress",                                   # Local
    "runbeam.partner.egress.name.partner-backend"      # To partner team
]
description = "Production mesh with partner integration"
enabled = true
```

In this example:
- `api-ingress` is defined locally in a pipeline file
- `partner-ingress` is managed by the partner team on Runbeam Cloud
- `backend-egress` is defined locally
- `partner-backend` is managed by the partner team on Runbeam Cloud

## Reference Resolution Process

References are resolved at different times depending on context:

### Startup Resolution

When Harmony starts:
1. Validates that all referenced providers exist in configuration
2. Validates reference syntax is correct
3. Does NOT fetch remote references yet

Remote references are resolved on-demand when needed.

### Runtime Resolution

When a mesh needs to use a referenced resource:
1. Harmony parses the reference string
2. Looks up the provider configuration (e.g., `[provider.runbeam]`)
3. Calls the provider's API to resolve the resource
4. Caches the result for subsequent requests

### Resolution Caching

Resolved references are cached to avoid repeated API calls:
- Cache duration depends on provider configuration
- Configuration changes trigger cache invalidation
- Failed resolutions are not cached (retry on next request)

## Validation Rules

Harmony validates references at configuration load time:

### Valid References

```toml
ingress = [
    "local-ingress",                                # ✓ Bare name (local)
    "local.name.local-ingress",                     # ✓ Explicit local reference
    "runbeam.my-team.ingress.name.api-ingress",     # ✓ Full provider reference by name
    "runbeam.id.abc123"                             # ✓ Provider reference by ID
]
```

### Invalid References

```toml
ingress = [
    "unknown.team.ingress.name.api",    # ✗ Unknown provider (not configured)
    "runbeam.team.invalid.name.api",    # ✗ Invalid resource type
    "runbeam.team.ingress",             # ✗ Incomplete reference
    "my-ingress.extra",                 # ✗ Extra components
]
```

Invalid references cause configuration validation to fail at startup with descriptive error messages.

## Error Handling

When reference resolution fails at runtime:

| Error | Cause | Result |
|-------|-------|--------|
| Provider unavailable | API unreachable | Request fails with 503 Service Unavailable |
| Resource not found | Resource doesn't exist | Request fails with 404 Not Found |
| Permission denied | No access to resource | Request fails with 403 Forbidden |
| Invalid reference | Malformed reference string | Configuration fails to load at startup |
| Token expired | Authorization token no longer valid | Request fails with 401 Unauthorized (re-auth needed) |

## Best Practices

### Use Local References When Possible

Local references are faster and don't require network calls:

```toml
# Prefer bare names for local resources
ingress = ["my-local-ingress"]

# Use provider references only for external resources
ingress = ["runbeam.partner.ingress.name.their-api"]
```

### Validate Provider Configuration

Ensure providers are configured before using references:

```toml
# config.toml
[provider.runbeam]
api = "https://api.runbeam.cloud"
poll_interval_secs = 30

# mesh/production.toml - can now use runbeam references
[mesh.production]
ingress = ["runbeam.partner.ingress.name.api"]
```

### Document Cross-Team References

When referencing resources from other teams, document the dependency:

```toml
# mesh/partner-integration.toml
# Depends on: partner-team's api-ingress (contact: partner@example.com)
[mesh.partner-integration]
type = "http3"
provider = "runbeam"
ingress = ["runbeam.partner-team.ingress.name.api-ingress"]
egress = ["runbeam.partner-team.egress.name.partner-backend"]
```

### Use Consistent Naming

Use clear, descriptive names for resources:

```toml
# Good: Clear what this is
ingress = ["fhir-ingress", "dicom-ingress"]
egress = ["partner-fhir-egress", "partner-dicom-egress"]

# Avoid: Unclear abbreviations
ingress = ["ing-1", "ing-2"]
egress = ["eg-1", "eg-2"]
```

### Plan for Team Changes

When referencing resources from other teams, be aware that:
- Team names may change
- Resource ownership may transfer
- Resource IDs should be preferred over names for stability

## Reference Syntax in Other Contexts

While references are primarily used in mesh configurations, they can be used in other contexts that support provider resolution.

Always verify the specific context to understand which reference formats are supported.

## Troubleshooting

**"Unknown provider in reference"**
- Check that the provider is configured in `[provider.*]`
- Verify the provider name matches exactly (case-sensitive)
- Check the primary_provider setting

**"Invalid reference syntax"**
- Verify the reference format matches one of the documented patterns
- Check for typos in provider, team, type names
- Ensure components are separated by correct number of dots

**"Permission denied resolving reference"**
- Verify the gateway has permission to access the remote resource
- Check authorization tokens are valid (may need re-auth)
- Confirm the resource exists and is accessible to the team

**"Reference resolution timeout"**
- Check network connectivity to provider API
- Verify provider API is running and accessible
- Consider increasing timeout if API is slow

## See Also

- [Mesh Configuration](./mesh.md) - Using references in mesh configurations
- [Providers](./providers.md) - Provider configuration and setup
- [Configuration Schemas](./schema.md) - Reference syntax in schemas
- [Configuration](./configuration.md) - Main configuration documentation
