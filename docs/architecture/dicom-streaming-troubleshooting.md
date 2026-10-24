---
title: DICOM Streaming Errors, Testing, and Troubleshooting
description: How DICOM streaming handles errors, how it is tested, and how to diagnose problems.
status: active
updated: 2026-10-24
type: Architecture
related:
  - architecture/dicom-streaming.md
---

# DICOM Streaming Errors, Testing, and Troubleshooting

How DICOM streaming handles errors, how it is tested, and how to diagnose problems. This page is part of [DICOM Streaming Architecture](dicom-streaming.md).

## Error Handling

### Per-Instance Errors

Failed instances are logged and skipped. Stream continues.

**Log Output**:
```
WARN Instance 47 failed transform: Invalid DICOM tag 00111111
INFO Continuing stream, 46 successful, 1 failed
```

**Error Envelope** (SSE format):
```
event: error
data: {"_sequence":47,"_error":"Transform failed","_sop_instance_uid":"1.2.3..."}
```

### Stream Termination

Stream terminates on:
- Backend connection failure
- Client disconnect
- Unrecoverable protocol error

**Log Output**:
```
ERROR Backend connection lost: Connection reset by peer
INFO Stream terminated after 123 instances (120 successful, 3 failed)
```

### Retry Logic (TODO)

Automatic retry for:
- Transient network errors
- Backend timeouts
- HTTP 5xx responses

```toml
[backends.pacs_streaming.options]
retry_attempts = 3
retry_delay_ms = 1000
retry_backoff_multiplier = 2.0
```

## Testing

### Unit Tests

Located in `src/models/services/types/`:
- `dicom_scu_streaming_test.rs`
- `http_streaming_test.rs`
- `http_streaming_backend_test.rs`

```bash
cargo test --lib streaming
```

### Integration Tests

Located in `tests/streaming_integration.rs`:

```bash
# Full streaming test suite
cargo test --test streaming_integration

# Specific test
cargo test --test streaming_integration test_dicom_to_http_stream
```

### Manual Testing

**Start Streaming Proxy**:
```bash
cargo run -- --config examples/dicom-streaming/bidirectional-streaming.toml
```

**Trigger Stream (C-GET)**:
```bash
# Using dcmtk tools
getscu -v -aet HARMONY_CLIENT -aec SENDER_SCP localhost 11112 \
  -k "StudyInstanceUID=1.2.840.113619.2.55.3.2609295819.820.1299003904.1"
```

**Monitor Stream (HTTP)**:
```bash
curl -N http://localhost:8080/dicom-stream \
  -H "Accept: application/x-ndjson" \
  -d '{"StudyInstanceUID":"1.2.840..."}'
```

**Expected Output**:
```json
{"_sequence":1,"_sop_instance_uid":"1.2.3.4.5.6.7.1","00080016":{"vr":"UI"...}}
{"_sequence":2,"_sop_instance_uid":"1.2.3.4.5.6.7.2","00080016":{"vr":"UI"...}}
...
```

## Troubleshooting

### "No instances streamed"

**Symptom**: Stream completes but no instances received

**Causes**:
1. Query returned no results
2. Backend not configured as streaming service
3. Middleware rejecting all instances

**Debug**:
```bash
RUST_LOG=harmony=debug,harmony::models::services::types::dicom_scu_streaming=trace cargo run
```

**Check Logs For**:
```
DEBUG Started DICOM C-GET operation
DEBUG Received instance 1: 2048 bytes
ERROR Transform middleware rejected instance (missing required field)
```

### "Memory still growing"

**Symptom**: Memory usage increases with study size despite streaming

**Causes**:
1. Using non-streaming backend (`dicom_scu` instead of `dicom_scu_streaming`)
2. Middleware buffering instances (custom middleware issue)
3. A backend or endpoint that collects results before returning them

**Verify Streaming Backend**:
```bash
grep -A5 "backends.pacs" config.toml
# Should show: service = "dicom_scu_streaming"
```

### "Stream hangs after N instances"

**Symptom**: Stream stops progressing mid-study

**Causes**:
1. Backend connection timeout
2. Middleware deadlock (unlikely)
3. Client disconnect not detected

**Increase Timeout**:
```toml
[backends.pacs_streaming.options]
connection_timeout_secs = 300  # Default: 60
```

### "Transform errors breaking stream"

**Symptom**: Stream terminates on first transform error

**Cause**: `fail_on_error = true` in middleware config

**Solution**:
```toml
[middleware.my_transform.options]
fail_on_error = false  # Continue on transform errors
```
