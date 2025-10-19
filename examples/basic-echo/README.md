# Basic Echo Example

This example demonstrates a simple HTTP passthrough pipeline with an echo backend. It's the simplest possible Harmony configuration and serves as a quick-start reference.

## What This Example Demonstrates

- Basic HTTP endpoint configuration
- Passthrough middleware
- Echo backend (returns the request data as the response)
- Minimal configuration required to run Harmony

## Prerequisites

None - this example has no external dependencies.

## Configuration

- **Proxy ID**: `harmony-basic-echo`
- **HTTP Listener**: `127.0.0.1:8080`
- **Endpoint Path**: `/echo`
- **Log File**: `./tmp/harmony_basic_echo.log`
- **Storage**: `./tmp`

## How to Run

1. From the project root, run:
   ```bash
   cargo run -- --config examples/basic-echo/config.toml
   ```

2. The service will start and bind to `127.0.0.1:8080`

## Testing

Send a simple HTTP request to the echo endpoint:

```bash
# Basic GET request
curl -v http://127.0.0.1:8080/echo

# POST with JSON data
curl -X POST http://127.0.0.1:8080/echo \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, Harmony!"}'
```

### Expected Behavior

The echo backend will return the request data back to you. You should see:
- Request headers
- Request body
- Metadata about the request

The response demonstrates how Harmony processes and transforms requests through its pipeline.

## Files

- `config.toml` - Main configuration file
- `pipelines/basic-echo.toml` - Pipeline definition
- `tmp/` - Created at runtime for logs and temporary storage

## Next Steps

After understanding this basic example, explore:
- `examples/fhir/` - Authentication and JSON extraction
- `examples/transform/` - Data transformation with JOLT
- `examples/dicom-backend/` - DICOM protocol integration
