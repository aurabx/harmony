# Debugging Pipeline Issues in Harmony

## Reference Documentation
- `docs/router.md` - Pipeline architecture
- `docs/middleware.md` - Middleware behavior
- `docs/backends.md` - Backend configuration

## Request Flow

```
Protocol Adapter (HTTP/DIMSE/etc.)
    ↓
ProtocolCtx + RequestEnvelope
    ↓
Incoming Middleware (auth, transform, filter)
    ↓
Backend Service (HTTP, FHIR, DICOM, etc.)
    ↓
Outgoing Middleware (response transform)
    ↓
ResponseEnvelope
    ↓
Protocol Adapter → Response
```

## Enable Debug Logging

### Via environment variable
```bash
RUST_LOG=harmony=debug cargo run -- --config config.toml
```

### Via config.toml
```toml
[logging]
level = "debug"
# Or for specific modules:
# level = "harmony::pipeline=debug,harmony::adapters=info"
```

## Common Issues & Solutions

### 1. Request Not Reaching Backend

**Symptoms**: No backend logs, immediate error response

**Check**:
- Middleware rejecting request (auth failure, path filter)
- Route not matching endpoint

**Debug**:
```bash
RUST_LOG=harmony::pipeline::middleware=debug cargo run
```

Look for:
- `Middleware rejected request`
- `PathDenied` errors
- `Authentication failed`

### 2. 401 Unauthorized

**Cause**: JWT or Basic auth middleware rejecting

**Check**:
- Token present in `Authorization` header
- Token not expired (`exp` claim)
- Correct issuer/audience if configured
- Public key path exists (for RS256)

**Debug**:
```bash
RUST_LOG=harmony::pipeline::middleware::jwt=debug cargo run
```

### 3. 404 Not Found

**Possible causes**:
1. **Path filter denying**: Check `path_filter` middleware rules
2. **No matching endpoint**: Check endpoint `path_prefix`
3. **Backend returning 404**: Check backend target URL

**Debug**: Check which layer is returning 404:
```bash
RUST_LOG=harmony=debug cargo run
```

### 4. Transform Not Applied

**Check**:
- `spec_path` points to valid JSON file
- `apply_to` is correct (`request`, `response`, or `both`)
- Transform middleware is in pipeline's `middleware` list
- Middleware order (transforms run in order listed)

**Debug**:
```bash
RUST_LOG=harmony::pipeline::middleware::transform=debug cargo run
```

### 5. Backend Connection Failed

**Symptoms**: Timeout or connection refused

**Check**:
- Backend `base_url` or `host`/`port` correct
- Target is reachable from Harmony host
- TLS certificates valid (for HTTPS/HTTP3)
- Firewall/network allows connection

**Test connectivity**:
```bash
curl -v https://your-backend-url/health
```

### 6. Wrong Response Data

**Check**:
- Response transform modifying data unexpectedly
- Backend returning different format than expected
- Content-type handling

**Debug with echo backend**:
```toml
[backends.debug_echo]
service = "echo"
```
Route through echo to see exact request reaching backend.

## Useful Debug Commands

### Validate config without starting
```bash
./harmony --config config.toml --validate-config
```

### Test specific endpoint
```bash
curl -v http://localhost:8080/your/endpoint \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"test": "data"}'
```

### Check loaded pipelines
Look at startup logs for:
```
Loaded pipeline: my_pipeline
  Networks: [default]
  Endpoints: [my_endpoint]
  Backends: [my_backend]
  Middleware: [auth, transform]
```

## Request Tracing

Each request gets a trace ID. Look for it in logs:
```
[trace_id=abc123] Processing request to /api/endpoint
[trace_id=abc123] Middleware auth: passed
[trace_id=abc123] Backend http: calling https://api.example.com
[trace_id=abc123] Backend http: response 200 in 45ms
```

## Integration Test Debugging

Run specific test with output:
```bash
cargo test --test http_backend -- --nocapture
```

Run with debug logging:
```bash
RUST_LOG=debug cargo test --test http_backend -- --nocapture
```
