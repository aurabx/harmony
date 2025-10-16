# Management API

The Management API provides administrative endpoints for monitoring and inspecting the Harmony proxy at runtime. It is disabled by default for security and must be explicitly enabled in the configuration.

## Configuration

The Management API is configured through the `[management]` section in your config file:

```toml
[management]
# Whether the management API is enabled. Defaults to false for security.
enabled = true
# The base path for management endpoints. Defaults to "admin".
base_path = "admin"
# The network to use for management endpoints. Required when enabled.
network = "management"
```

### Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `false` | Whether the management API is enabled |
| `base_path` | string | `"admin"` | Base path for all management endpoints |
| `network` | string | none | Network name to bind management endpoints to (required when enabled) |

## Automatic Service Injection

When the management API is enabled (`enabled = true`), Harmony automatically injects:

- A management endpoint of type `management`
- A management pipeline connected to the specified network
- The service registration for the management endpoint

No manual pipeline or endpoint configuration is required - everything is handled automatically based on the management configuration. You must specify which network to use via the `network` configuration option.

## Endpoints

All endpoints are prefixed with the configured `base_path` (default: `admin`).

### GET /{base_path}/info

Returns basic system information about the Harmony proxy instance.

**Example Request:**
```bash
# Management API runs on internal network (localhost:9090)
curl http://localhost:9090/admin/info
```

**Response:**
```json
{
  "version": "0.1.0",
  "uptime": 1734307200,
  "os": "macos",
  "arch": "aarch64"
}
```

**Response Fields:**
- `version`: Harmony proxy version from Cargo.toml
- `uptime`: Unix timestamp when the service started
- `os`: Operating system (linux, macos, windows, etc.)
- `arch`: System architecture (x86_64, aarch64, etc.)

### GET /{base_path}/pipelines

Returns a list of all configured pipelines in the system.

**Example Request:**
```bash
# Management API runs on internal network (localhost:9090)
curl http://localhost:9090/admin/pipelines
```

**Response:**
```json
{
  "pipelines": [
    {
      "id": "management",
      "description": "Management API pipeline",
      "networks": ["default"],
      "endpoints": ["management"],
      "backends": [],
      "middleware": []
    },
    {
      "id": "my-custom-pipeline",
      "description": "Custom pipeline for FHIR processing",
      "networks": ["default"],
      "endpoints": ["fhir-endpoint"],
      "backends": ["fhir-backend"],
      "middleware": ["jwt_auth", "transform"]
    }
  ]
}
```

**Response Fields:**
- `pipelines`: Array of pipeline objects
  - `id`: Unique pipeline identifier
  - `description`: Human-readable description
  - `networks`: Networks this pipeline is associated with
  - `endpoints`: List of endpoint names used by this pipeline
  - `backends`: List of backend names used by this pipeline
  - `middleware`: Ordered list of middleware applied to requests

## Security Considerations

### Default Disabled
The Management API is disabled by default (`enabled = false`) to prevent accidental exposure of system information. You must explicitly enable it.

### Network Security
The recommended configuration binds the Management API to the management network (`127.0.0.1:9090`), making it accessible only from the local machine. This provides a security boundary between:

- **Management network** (`127.0.0.1:9090`): Management API endpoints only
- **External network** (`0.0.0.0:8080`): Client-facing application endpoints

### No Authentication (Yet)
The current implementation does not include authentication within the management endpoints themselves. The network-level isolation provides the primary security boundary. Consider additional measures:

- Network-level restrictions (firewall, VPN) for remote management access
- Planning to add authentication in future versions

### Information Disclosure
The API exposes:
- System configuration details (pipelines, endpoints, backends)
- Runtime information (version, OS, architecture)
- No sensitive data like keys, tokens, or user data

## Usage Examples

### Check System Health
```bash
# Get basic system info (note: management API on port 9090)
curl http://localhost:9090/admin/info | jq

# Check if specific pipeline exists
curl http://localhost:9090/admin/pipelines | jq '.pipelines[] | select(.id=="my-pipeline")'

# Count total pipelines
curl http://localhost:9090/admin/pipelines | jq '.pipelines | length'
```

### Monitoring Integration
The endpoints return JSON suitable for monitoring tools:

```bash
# Prometheus-style check (non-zero exit on error)
curl -f http://localhost:9090/admin/info > /dev/null

# Extract version for deployment tracking
VERSION=$(curl -s http://localhost:9090/admin/info | jq -r '.version')
echo "Running Harmony version: $VERSION"
```

### Pipeline Discovery
```bash
# List all pipeline IDs
curl -s http://localhost:9090/admin/pipelines | jq -r '.pipelines[].id'

# Find pipelines using specific middleware
curl -s http://localhost:9090/admin/pipelines | \
  jq '.pipelines[] | select(.middleware | contains(["jwt_auth"]))'

# Check pipeline network associations
curl -s http://localhost:9090/admin/pipelines | \
  jq '.pipelines[] | {id, networks}'
```

## Error Responses

### Management API Disabled
If the management API is disabled, requests to management endpoints will return 404 Not Found since the pipeline is not created.

### Invalid Endpoints
Requests to non-existent management endpoints return:
```json
{"error": "Not found"}
```

### Service Unavailable
If the system is under heavy load or experiencing issues, endpoints may return 500 Internal Server Error with error details.

## Integration with Existing Systems

### Health Checks
Use the `/info` endpoint for health checks in container orchestration:

```yaml
# Docker Compose (note: management API on port 9090)
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:9090/admin/info"]
  interval: 30s
  timeout: 10s
  retries: 3
```

```yaml
# Kubernetes (note: management API on port 9090)
livenessProbe:
  httpGet:
    path: /admin/info
    port: 9090
  initialDelaySeconds: 30
  periodSeconds: 10
```

### Configuration Validation
Use the `/pipelines` endpoint to validate configuration after deployment:

```bash
#!/bin/bash
# Deployment validation script
EXPECTED_PIPELINES=("pipeline1" "pipeline2" "management")
ACTUAL_PIPELINES=$(curl -s http://localhost:9090/admin/pipelines | jq -r '.pipelines[].id')

for pipeline in "${EXPECTED_PIPELINES[@]}"; do
  if ! echo "$ACTUAL_PIPELINES" | grep -q "^$pipeline$"; then
    echo "ERROR: Pipeline $pipeline not found"
    exit 1
  fi
done
echo "All expected pipelines are configured"
```

## Future Enhancements

Planned additions to the Management API:

- **Authentication**: JWT or API key-based authentication
- **Metrics**: Runtime performance metrics and counters  
- **Configuration**: Dynamic configuration updates
- **Network Status**: Network and connection health information
- **Middleware Inspection**: Middleware configuration and status
- **Real-time Updates**: WebSocket endpoints for live monitoring