# Management API

**Last Updated**: 2025-01-18 (Phase 6)

The Management API provides administrative endpoints for monitoring and inspecting the Harmony proxy at runtime. It is disabled by default for security and must be explicitly enabled in the configuration.

**Note**: The Management API continues to work unchanged in Phase 6. It runs through the same `HttpAdapter` and `PipelineExecutor` as other HTTP endpoints, but is isolated on a dedicated management network.

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

### GET /{base_path}/routes

Returns a list of all configured routes in the system, showing which endpoints and pipelines handle each route.

**Example Request:**
```bash
# Management API runs on internal network (localhost:9090)
curl http://localhost:9090/admin/routes
```

**Response:**
```json
{
  "routes": [
    {
      "path": "/admin/info",
      "methods": ["GET"],
      "description": "Get system information",
      "endpoint_name": "management",
      "service_type": "management",
      "pipeline": "management"
    },
    {
      "path": "/fhir/*",
      "methods": ["GET", "POST", "PUT", "DELETE"],
      "description": "FHIR endpoint for resource operations",
      "endpoint_name": "fhir-endpoint",
      "service_type": "fhir",
      "pipeline": "fhir-pipeline"
    }
  ]
}
```

**Response Fields:**
- `routes`: Array of route objects
  - `path`: The route path pattern
  - `methods`: HTTP methods supported by this route
  - `description`: Human-readable description of the route
  - `endpoint_name`: Name of the endpoint handling this route
  - `service_type`: Type of service (e.g., "fhir", "management")
  - `pipeline`: Name of the pipeline containing this route

### POST /{base_path}/authorize

Authorize the Harmony gateway with Runbeam Cloud and obtain a machine-scoped token for autonomous API access.

This endpoint implements the gateway authorization flow:
1. Validates the user's JWT token from the Authorization header
2. Calls Runbeam Cloud API to exchange the user token for a machine token
3. Stores the machine token locally for future API calls
4. Returns gateway details and token expiry information

**Authentication Required:** Yes (JWT Bearer token from Runbeam Cloud)

**Example Request:**
```bash
# Authorize gateway with Runbeam Cloud
curl -X POST http://localhost:9090/admin/authorize \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..." \
  -H "Content-Type: application/json" \
  -d '{
    "gateway_code": "harmony-prod-01",
    "machine_public_key": "ssh-rsa AAAAB3...",
    "metadata": {
      "version": "0.4.0",
      "os": "linux",
      "arch": "x86_64"
    }
  }'
```

**Request Body:**
```json
{
  "gateway_code": "harmony-prod-01",
  "machine_public_key": "optional-public-key",
  "metadata": {
    "version": "0.4.0",
    "os": "linux"
  }
}
```

**Request Fields:**
- `gateway_code` (required): Gateway instance ID or code
- `machine_public_key` (optional): Public key for secure communication
- `metadata` (optional): Additional gateway metadata (version, OS, etc.)

**Success Response (201 Created):**
```json
{
  "success": true,
  "message": "Gateway authorized successfully",
  "gateway": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "code": "harmony-prod-01",
    "name": "Gateway harmony-prod-01"
  },
  "expires_at": "2025-11-24T12:48:46Z",
  "expires_in": 2592000
}
```

**Response Fields:**
- `success`: Boolean indicating success
- `message`: Human-readable status message
- `gateway`: Gateway details
  - `id`: Unique gateway ID from Runbeam Cloud
  - `code`: Gateway code (instance ID)
  - `name`: Human-readable gateway name
- `expires_at`: ISO 8601 timestamp when machine token expires
- `expires_in`: Seconds until token expiry (typically 30 days = 2,592,000 seconds)

**Error Responses:**

**400 Bad Request** - Invalid request body:
```json
{
  "error": "Bad Request",
  "message": "Invalid request body: missing field `gateway_code`"
}
```

**401 Unauthorized** - Missing or invalid JWT token:
```json
{
  "error": "Unauthorized",
  "message": "Invalid or expired token: Token validation failed"
}
```

**403 Forbidden** - User not authorized for this gateway:
```json
{
  "error": "Forbidden",
  "message": "This gateway belongs to a different team"
}
```

**500 Internal Server Error** - Runbeam Cloud API error:
```json
{
  "error": "Internal Server Error",
  "message": "Authorization failed: Network error"
}
```

**Notes:**
- The JWT token is obtained via the `runbeam login` CLI command
- The JWT secret must match what Runbeam Cloud uses (set via `RUNBEAM_JWT_SECRET` env var)
- The machine token is stored at `./tmp/runbeam/auth.json` by default
- Machine tokens expire after 30 days and must be renewed
- The Runbeam API base URL is extracted from the JWT's `iss` (issuer) claim

## Security Considerations

### Default Disabled
The Management API is disabled by default (`enabled = false`) to prevent accidental exposure of system information. You must explicitly enable it.

### Network Security
The recommended configuration binds the Management API to the management network (`127.0.0.1:9090`), making it accessible only from the local machine. This provides a security boundary between:

- **Management network** (`127.0.0.1:9090`): Management API endpoints only
- **External network** (`0.0.0.0:8080`): Client-facing application endpoints

### Authorization Endpoint Authentication
The `/authorize` endpoint requires JWT authentication:

- **JWT Token Required**: Must provide valid JWT from Runbeam Cloud in `Authorization: Bearer <token>` header
- **Token Validation**: Uses HS256 algorithm with shared secret (configured via `RUNBEAM_JWT_SECRET` environment variable)
- **Token Claims**: Extracts Runbeam API base URL from `iss` (issuer) claim
- **Machine Token**: Exchanges user JWT for 30-day machine-scoped token stored locally

### Other Management Endpoints
The info/pipelines/routes endpoints currently do not require authentication. Network-level isolation provides the primary security boundary. Consider additional measures:

- Network-level restrictions (firewall, VPN) for remote management access
- JWT authentication may be extended to all endpoints in future versions

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