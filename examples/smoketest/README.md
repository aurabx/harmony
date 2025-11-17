# Harmony Comprehensive Smoketest

This example provides a comprehensive integration test of all major Harmony features in a single pipeline.

## Features Tested

- ✅ **HTTP Endpoint & Backend** - HTTP service with passthrough to Python backend
- ✅ **Path Filtering** - Allow `/api/*` and `/health`, deny everything else
- ✅ **Content-Type Filtering** - Accept only `application/json`
- ✅ **Method Filtering** - Allow only GET and POST methods
- ✅ **Transform Middleware** - JOLT transformations on request and response
- ✅ **JSON Extraction** - Parse JSON request bodies
- ✅ **Access Control Policies** - IP allowlist and rate limiting
- ✅ **Basic Authentication** - Username/password auth
- ✅ **Management API** - Admin endpoints

## Quick Start

```bash
# Run the automated demo
./demo.sh
```

The demo script will:
1. Start a Python HTTP backend server
2. Build and start Harmony with the smoketest config
3. Run 7 test scenarios
4. Display results
5. Wait for you to press Enter before cleaning up

## Manual Testing

### Start Backend Server

```bash
mkdir -p tmp/backend
cd tmp/backend
python3 -m http.server 8888
```

### Start Harmony

```bash
cargo run --release -- --config examples/smoketest/config.toml
```

### Test Commands

**Valid Request (should succeed):**
```bash
curl -X POST http://localhost:8080/api/transform \
  -u testuser:testpass \
  -H "Content-Type: application/json" \
  -d '{"name": "John Doe", "id": "12345"}' | jq
```

**Invalid Path (should 404):**
```bash
curl -u testuser:testpass http://localhost:8080/invalid/path
```

**Invalid Method (should 403):**
```bash
curl -X DELETE -u testuser:testpass http://localhost:8080/api/transform
```

**Invalid Content-Type (should 403):**
```bash
curl -X POST http://localhost:8080/api/transform \
  -u testuser:testpass \
  -H "Content-Type: text/plain" \
  -d "plain text"
```

**No Auth (should 401):**
```bash
curl -X POST http://localhost:8080/api/transform \
  -H "Content-Type: application/json" \
  -d '{"test": "data"}'
```

**Management API:**
```bash
curl http://localhost:9090/admin/info
curl http://localhost:9090/admin/pipelines
```

## Request/Response Flow

### Input
```json
{
  "name": "John Doe",
  "id": "12345"
}
```

### After Request Transform
```json
{
  "data": {
    "user": {
      "fullName": "John Doe",
      "userId": "12345"
    },
    "metadata": {
      "transformedAt": "2024-01-01T00:00:00Z",
      "transformType": "request",
      "pipeline": "smoketest"
    }
  }
}
```

### After Response Transform
```json
{
  "result": {
    "transformedData": {
      "user": {
        "fullName": "John Doe",
        "userId": "12345"
      }
    },
    "requestMetadata": {
      "transformedAt": "2024-01-01T00:00:00Z",
      "transformType": "request",
      "pipeline": "smoketest"
    },
    "responseMetadata": {
      "transformedAt": "2024-01-01T00:00:00Z",
      "transformType": "response",
      "status": "success"
    }
  }
}
```

## Configuration Files

- `config.toml` - Main configuration with service/middleware registrations
- `pipelines/smoketest.toml` - Pipeline with all middleware configured
- `transforms/request_transform.json` - JOLT spec for request transformation
- `transforms/response_transform.json` - JOLT spec for response transformation

## Middleware Order

The pipeline processes requests through middleware in this order:

1. **basic_auth** - Verify credentials (testuser:testpass)
2. **access_policies** - Check IP, rate limits, content-type, method
3. **path_filter** - Ensure path is allowed
4. **json_extractor** - Parse JSON body
5. **request_transform** - Apply JOLT transform to request
6. **passthru** - Pass through to backend
7. **response_transform** - Apply JOLT transform to response

## Troubleshooting

**Check logs:**
```bash
tail -f tmp/smoketest.log
tail -f tmp/backend.log
```

**Verify backend is running:**
```bash
curl http://127.0.0.1:8888/
```

**Test management API:**
```bash
curl http://127.0.0.1:9090/admin/info
```

## Use as Template

This configuration serves as a comprehensive template for building production pipelines. Copy and modify to suit your needs:

```bash
cp -r examples/smoketest examples/my-pipeline
cd examples/my-pipeline
# Edit config files as needed
```
