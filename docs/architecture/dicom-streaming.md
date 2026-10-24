---
title: DICOM Streaming Architecture
description: How Harmony streams large DICOM studies through a pipeline one instance at a time.
status: active
updated: 2026-10-24
type: Architecture
---

# DICOM Streaming Architecture

## Overview

Harmony's streaming architecture enables efficient processing of large DICOM studies by treating each instance as an independent unit that flows through the pipeline. This eliminates memory bottlenecks, reduces latency, and enables progressive processing.

**Key Benefits:**
- **Constant Memory**: Process 1000+ instance studies with ~2MB memory overhead per instance
- **Low Latency**: First instance available within milliseconds of backend response
- **Progressive Processing**: Middleware operates on each instance independently
- **Fault Tolerance**: Per-instance error handling, failed instances don't block the stream

## Architecture

### Components

```
┌─────────────────────────────────────────────────────────────────┐
│                     STREAMING PIPELINE                           │
│                                                                  │
│  ┌──────────┐    ┌─────────────┐    ┌─────────────┐           │
│  │ Endpoint │───▶│ Middleware  │───▶│  Streaming  │           │
│  │          │    │  (Incoming) │    │   Backend   │           │
│  └──────────┘    └─────────────┘    └──────┬──────┘           │
│                                              │                   │
│                                              ▼                   │
│                                      Stream<Envelope>            │
│                                              │                   │
│                                              ▼                   │
│                         ┌────────────────────────────┐          │
│                         │  Per-Instance Processing   │          │
│                         │  ├─ Middleware (Outgoing) │          │
│                         │  └─ Endpoint Response     │          │
│                         └────────────────────────────┘          │
└─────────────────────────────────────────────────────────────────┘
```

### Flow

1. **Request Phase**: Single request envelope processed through endpoint and incoming middleware
2. **Backend Streaming**: Backend yields `Stream<ResponseEnvelope>` - one per DICOM instance
3. **Per-Instance Processing**: Each envelope independently through outgoing middleware
4. **Response Assembly**: Endpoint converts stream to appropriate protocol format

## DICOM Streaming Services

The endpoint and backend services that take part in DICOM streaming. See [DICOM Streaming Services](dicom-streaming-services.md).

## Pipeline Configuration

### Basic Streaming Pipeline

```toml
[pipelines.dicom_streaming]
description = "Stream DICOM instances with progressive processing"
networks = ["default"]
endpoints = ["http_api"]
backends = ["pacs_streaming"]
middleware = ["auth", "transform"]
```

**Execution Flow:**
1. HTTP request received on `http_api` endpoint
2. Request through `auth` middleware (incoming direction)
3. Request to `pacs_streaming` backend → returns Stream
4. **For each instance:**
   - Apply `auth` middleware (outgoing direction)
   - Apply `transform` middleware (outgoing direction)
   - Convert to HTTP stream format
5. HTTP response streamed back to client

### Bidirectional Streaming

**Proxy A (Sender):**
```toml
[endpoints.dicom_listener]
service = "dicom_scp"  # DIMSE endpoint

[backends.local_pacs]
service = "dicom_scu_streaming"

[endpoints.http_stream]
service = "http_streaming"

[pipelines.sender]
endpoints = ["dicom_listener"]
backends = ["local_pacs"]
# Streams to http_stream endpoint
```

**Proxy B (Receiver):**
```toml
[endpoints.http_api]
service = "http"

[backends.remote_stream]
service = "http_streaming_backend"  # Consumes stream from Proxy A

[backends.target_pacs]
service = "dicom_scu"

[pipelines.receiver]
endpoints = ["http_api"]
backends = ["remote_stream", "target_pacs"]
# Each instance forwarded to target_pacs
```

## Middleware Processing

### Per-Instance Middleware

All middleware operates **independently** on each streamed instance:

```toml
[middleware.transform_dicom]
type = "transform"

[middleware.transform_dicom.options]
spec_path = "transforms/anonymize.json"
apply = "right"  # Apply to backend responses
fail_on_error = false  # Continue stream on transform error
```

**Behavior:**
- Each ResponseEnvelope processed sequentially
- Transform applied to `normalized_data`
- Pre-transform snapshot available in `normalized_snapshot`
- Failed transforms logged, stream continues if `fail_on_error = false`

### Transform Examples

#### Extract Patient Demographics
```json
[
  {
    "operation": "shift",
    "spec": {
      "00100010": "patient.name",
      "00100020": "patient.id",
      "00100030": "patient.dob"
    }
  }
]
```

#### Anonymize DICOM Tags
```json
[
  {
    "operation": "remove",
    "spec": {
      "00100010": "",
      "00100020": "",
      "00100030": ""
    }
  },
  {
    "operation": "default",
    "spec": {
      "00100010": {"vr": "PN", "Value": ["ANONYMOUS"]},
      "00100020": {"vr": "LO", "Value": ["ANON-ID"]}
    }
  }
]
```

## DICOM Streaming Use Cases and Performance

Example DICOM streaming pipelines and their performance characteristics. See [DICOM Streaming Use Cases and Performance](dicom-streaming-use-cases.md).

## DICOM Streaming Errors, Testing, and Troubleshooting

How DICOM streaming handles errors, how it is tested, and how to diagnose problems. See [DICOM Streaming Errors, Testing, and Troubleshooting](dicom-streaming-troubleshooting.md).

## Future Enhancements

### Planned Features

1. **Backpressure Control**: Automatic flow control when client consumes slowly
2. **Adaptive Batching**: Group small instances for efficiency
3. **Parallel Streaming**: Multiple backend connections for higher throughput
4. **Resume Support**: Stream continuation after connection interrupts
5. **Stream Multiplexing**: Multiple concurrent streams over single HTTP/3 connection

### Experimental Features

**Chunked Instance Streaming**: Stream large instances in chunks

```toml
[backends.pacs_streaming.options]
enable_chunked_instances = true
chunk_size_bytes = 65536  # 64KB chunks
```

**Benefits**: 
- Sub-instance progressive processing
- Lower memory for huge instances (500MB+ raw pixel data)

## References

- [Envelope Architecture](envelope.md)
- [Pipeline Executor](router.md)
- [Protocol Adapters](adapters.md)
- [DICOM Services](../configuration/backends-dicom-storage.md#dicom-scu-service-class-user)
- [HTTP Streaming Standards](https://html.spec.whatwg.org/multipage/server-sent-events.html)
- [NDJSON Format](http://ndjson.org/)
