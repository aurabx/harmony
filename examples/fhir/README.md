# FHIR Passthrough Example

This example demonstrates a FHIR endpoint with basic authentication and JSON extraction middleware. It shows how to secure endpoints and extract data from JSON payloads.

## What This Example Demonstrates

- FHIR endpoint configuration
- Basic authentication middleware
- JSON extraction middleware
- Secure HTTP API patterns
- Echo backend for testing

## Prerequisites

None - this example uses an echo backend for testing. In production, you would configure a real FHIR server backend.

## Configuration

- **Proxy ID**: `harmony-fhir`
- **HTTP Listener**: `127.0.0.1:8081`
- **Endpoint Path**: `/test`
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

### With Authentication

```bash
# Using basic auth credentials
curl -v http://127.0.0.1:8081/test \
  -u test_user:test_password \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Patient",
    "id": "example",
    "name": [{
      "family": "Smith",
      "given": ["John"]
    }]
  }'
```

### Without Authentication (will fail)

```bash
# This should return 401 Unauthorized
curl -v http://127.0.0.1:8081/test \
  -H "Content-Type: application/json" \
  -d '{"test": "data"}'
```

## Expected Behavior

- Requests with valid credentials are processed through the pipeline
- JSON data is extracted and normalized
- The echo backend returns the processed request
- Requests without credentials are rejected with 401 Unauthorized

## Files

- `config.toml` - Main configuration file with authentication setup
- `pipelines/fhir.toml` - Pipeline definition with middleware chain
- `tmp/` - Created at runtime for logs and temporary storage

## Next Steps

- Explore `examples/fhir-to-dicom/` for FHIR to DICOM translation
- See `examples/transform/` for data transformation examples
