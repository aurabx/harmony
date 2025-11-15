# DICOM Streaming Examples

This directory contains example configurations demonstrating Harmony's streaming capabilities for DICOM operations.

## Overview

Traditional DICOM proxy implementations collect all instances from a C-GET operation before returning a response. For large studies (100+ instances, multiple GB), this creates:

- **High memory usage**: Entire study buffered in RAM
- **Long latency**: First instance not available until last instance received
- **No progressive processing**: Middleware can't process instances as they arrive

Harmony's streaming architecture solves these problems by:

1. **Yielding instances immediately**: Each DICOM instance becomes a separate `ResponseEnvelope`
2. **Independent middleware processing**: Transforms, auth checks, etc. apply per-instance
3. **Constant memory usage**: Only one instance in memory at a time
4. **Progressive delivery**: Client receives instances as they arrive from PACS

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                          Harmony Pipeline                        │
│                                                                  │
│  DIMSE SCU Backend          Middleware           HTTP Endpoint  │
│  (dicom_scu_streaming)     (Per-Instance)        (future)       │
│                                                                  │
│  ┌──────────────┐         ┌──────────┐         ┌─────────────┐ │
│  │ C-GET Stream │────────▶│Transform │────────▶│ SSE/NDJSON  │ │
│  │              │         │  Auth    │         │  Multipart  │ │
│  │ Instance 1   │         │  Filter  │         │             │ │
│  │ Instance 2   │         │   ...    │         │             │ │
│  │ Instance N   │         │          │         │             │ │
│  └──────────────┘         └──────────┘         └─────────────┘ │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Components

### 1. `dicom_scu_streaming` Backend

A streaming-enabled DICOM SCU backend that yields ResponseEnvelopes per instance.

**Key Features:**
- Implements `StreamingBackend` trait
- Supports C-GET and C-FIND operations
- Converts DICOM instances to JSON for middleware processing
- Handles DIMSE stream errors gracefully

**Configuration:**
```toml
[backends.pacs_streaming]
service = "dicom_scu_streaming"

[backends.pacs_streaming.options]
aet = "REMOTE_PACS"
host = "pacs.example.com"
port = 4242
local_aet = "HARMONY_STREAM"
```

### 2. Streaming Pipeline Executor

The `execute_streaming()` method in `PipelineExecutor` that:
- Accepts streaming backends
- Applies middleware to each envelope in the stream independently
- Returns a stream of processed envelopes
- Integrated with adapters via service type detection

### 3. HTTP Streaming Endpoint

The `http_streaming` endpoint service converts ResponseEnvelope streams to HTTP streaming formats:

**Server-Sent Events (SSE):**
```
Content-Type: text/event-stream

data: {"SOPInstanceUID": "1.2.3.4.5.6.1", ...}

data: {"SOPInstanceUID": "1.2.3.4.5.6.2", ...}

data: {"SOPInstanceUID": "1.2.3.4.5.6.3", ...}
```

**Newline-Delimited JSON (NDJSON):**
```
Content-Type: application/x-ndjson

{"SOPInstanceUID": "1.2.3.4.5.6.1", ...}
{"SOPInstanceUID": "1.2.3.4.5.6.2", ...}
{"SOPInstanceUID": "1.2.3.4.5.6.3", ...}
```

**Multipart:**
```
Content-Type: multipart/x-mixed-replace; boundary=harmony-stream

--harmony-stream
Content-Type: application/dicom

[DICOM bytes instance 1]
--harmony-stream
Content-Type: application/dicom

[DICOM bytes instance 2]
--harmony-stream--
```

## Configuration Examples

### Basic Streaming

```toml
[pipelines.dicom_streaming]
description = "Stream DICOM instances from PACS"
networks = ["default"]
endpoints = ["http_api"]
backends = ["pacs_streaming"]
middleware = []
```

### With Per-Instance Transforms

```toml
[pipelines.dicom_streaming_transform]
description = "Stream with transforms applied per-instance"
networks = ["default"]
endpoints = ["http_api"]
backends = ["pacs_streaming"]
middleware = ["transform_dicom"]

[middleware.transform_dicom]
type = "transform"
[middleware.transform_dicom.options]
spec_path = "transforms/dicom_to_fhir.json"
apply = "right"
```

### With Authentication & Filtering

```toml
[pipelines.dicom_streaming_secure]
description = "Authenticated streaming with filtering"
networks = ["default"]
endpoints = ["http_api"]
backends = ["pacs_streaming"]
middleware = ["jwt_auth", "path_filter", "transform_dicom"]
```

## Usage Examples

### Start the Proxy

```bash
cargo run -- --config examples/dicom-streaming/streaming-config.toml
```

### Stream DICOM Instances (C-GET)

```bash
curl -X POST http://localhost:8080/dimse-streaming \
  -H "Content-Type: application/json" \
  -d '{
    "0020000D": {
      "vr": "UI",
      "Value": ["1.2.840.113619.2.55.3.604688119.868.1234567890.1"]
    }
  }'
```

### Stream with Query Parameters

```bash
curl -X POST http://localhost:8080/dimse-streaming \
  -H "Content-Type: application/json" \
  -d '{
    "0020000D": {"vr": "UI", "Value": ["1.2.3.4.5.6"]},
    "0020000E": {"vr": "UI", "Value": []}
  }'
```

## Benefits

### Memory Efficiency

**Before (Non-Streaming):**
- 100 instances × 10MB each = 1GB RAM usage
- All instances buffered before response

**After (Streaming):**
- ~10MB RAM usage (1 instance at a time)
- Constant memory regardless of study size

### Latency Reduction

**Before:**
- Wait for all 100 instances (10+ seconds)
- Client receives first instance after 10+ seconds

**After:**
- First instance available immediately (~100ms)
- Progressive delivery as instances arrive

### Progressive Processing

**Middleware per-instance:**
- Transform: Convert DICOM → FHIR per instance
- Auth: Check permissions per instance
- Filter: Drop instances based on criteria
- Aggregate: Build index as instances arrive

## Implementation Status

- ✅ **Streaming Backend**: `dicom_scu_streaming` implemented and compiling
- ✅ **HTTP Streaming Backend**: `http_streaming_backend` for consuming HTTP streams
- ✅ **Streaming Trait**: `StreamingBackend` trait defined
- ✅ **Service Registration**: All streaming services integrated with factory
- ✅ **Pipeline Executor**: `execute_streaming()` method implemented
- ✅ **HTTP Streaming Endpoint**: `http_streaming` service implemented (SSE/NDJSON/Multipart)
- ✅ **Bidirectional Configuration**: Complete proxy-to-proxy example
- ✅ **Documentation**: Comprehensive streaming architecture docs
- 🚧 **Adapter Integration**: Wire HTTP/DIMSE adapters to call execute_streaming
- 🚧 **Integration Tests**: End-to-end streaming tests with real PACS
- 🚧 **Performance Benchmarks**: Memory and latency measurements

## Next Steps

1. Wire HTTP adapter to detect streaming backends and call `execute_streaming()`
2. Wire DIMSE adapter to detect streaming backends
3. Create integration tests with Orthanc PACS
4. Performance benchmarks (memory profiling, latency analysis)
5. Backpressure and retry logic (see docs/streaming.md for TODOs)

## Performance Considerations

### Backpressure

The streaming architecture naturally supports backpressure:
- If client is slow, DIMSE stream pauses
- If middleware is slow, upstream buffers
- No unbounded memory growth

### Buffer Sizes

Configure buffering in the endpoint:

```toml
[endpoints.http_stream]
service = "http_streaming"
[endpoints.http_stream.options]
buffer_size = 10  # Buffer up to 10 instances
```

### Middleware Cost

Per-instance middleware processing adds latency:
- Simple transforms: ~1-5ms per instance
- Complex JOLT transforms: ~10-50ms per instance
- Auth checks: ~1-10ms per instance

For 100 instances with 5ms middleware:
- Total latency: 500ms added
- Still better than non-streaming (10+ seconds)

## Full Configuration Examples

See the complete bidirectional streaming configuration:
- [bidirectional-streaming.toml](bidirectional-streaming.toml) - Full sender/receiver proxy setup
- Demonstrates DIMSE → HTTP streaming (sender side)
- Demonstrates HTTP → DIMSE forwarding (receiver side)
- Includes middleware (auth, transforms) on both sides

## See Also

- [docs/streaming.md](../../docs/streaming.md) - **Complete streaming architecture guide**
- [docs/backends.md](../../docs/backends.md) - Backend documentation
- [docs/middleware.md](../../docs/middleware.md) - Middleware guide
- [docs/envelope.md](../../docs/envelope.md) - Envelope specification
- [src/models/services/streaming.rs](../../src/models/services/streaming.rs) - Streaming trait implementation
