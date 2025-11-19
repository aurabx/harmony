# FHIR Passthrough Example

This example demonstrates a FHIR endpoint that proxies to a real FHIR server with basic authentication and JSON extraction middleware. It shows how to secure endpoints and proxy FHIR resources through Harmony.

## What This Example Demonstrates

- FHIR endpoint configuration
- Basic authentication middleware
- JSON extraction middleware
- Proxying to external FHIR servers (Firely test server)
- Real FHIR-compliant responses (Patient, Observation, etc.)

## Prerequisites

None - this example uses the public Firely FHIR test server (https://server.fire.ly) as the backend, so no local FHIR server setup is required.

## Configuration

- **Proxy ID**: `harmony-fhir`
- **HTTP Listener**: `127.0.0.1:8081`
- **Endpoint Path**: `/fhir`
- **Backend**: `https://server.fire.ly` (Firely public test server)
- **Authentication**: Basic auth (username: `test_user`, password: `test_password`)
- **Log File**: `./tmp/harmony_fhir.log`
- **Storage**: `./tmp`

## How to Run

1. From the project root, run:
   ```bash
   cargo run -- --config examples/fhir/config.toml
   ```

2. The service will start and bind to `127.0.0.1:8081`

## Testing

### Search for Patients (with authentication)

```bash
# Search for patients (returns first 10 results)
curl http://127.0.0.1:8081/fhir/Patient?_count=10 \
  -u test_user:test_password \
  -H "Accept: application/fhir+json"
```

### Read a Specific Patient

```bash
# Get a specific patient by ID
curl -v http://127.0.0.1:8081/fhir/Patient/98eed96b-a738-49d4-b8b6-f5b9008a45ec \
  -u test_user:test_password \
  -H "Accept: application/fhir+json"
```

### List Recent Patients

```bash
# Get the first 5 patients
curl -v http://127.0.0.1:8081/fhir/Patient?_count=5 \
  -u test_user:test_password \
  -H "Accept: application/fhir+json"
```

### Without Authentication (will fail)

```bash
# This should return 401 Unauthorized
curl -v http://127.0.0.1:8081/fhir/Patient \
  -H "Accept: application/fhir+json"
```

## Expected Behavior

- Requests with valid credentials are proxied to the Firely FHIR test server
- Responses are valid FHIR Bundle resources (for searches) or individual resources (for reads)
- JSON data is extracted and normalized by the middleware
- Requests without credentials are rejected with 401 Unauthorized
- All responses conform to FHIR R4 specification

## Files

- `config.toml` - Main configuration file with authentication setup
- `pipelines/fhir.toml` - Pipeline definition with middleware chain
- `tmp/` - Created at runtime for logs and temporary storage

## Backend Server

This example uses the public Firely FHIR test server:
- **URL**: https://server.fire.ly
- **Version**: FHIR R4
- **Resources**: Patient, Observation, Practitioner, and more
- **Public Access**: No registration required

## Next Steps

- Explore `examples/fhir_dicom/` for FHIR to DICOM translation
- See `examples/transform/` for data transformation examples
- Try different FHIR resource types: `/fhir/Observation`, `/fhir/Practitioner`, etc.
