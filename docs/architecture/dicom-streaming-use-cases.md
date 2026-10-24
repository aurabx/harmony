---
title: DICOM Streaming Use Cases and Performance
description: Example DICOM streaming pipelines and their performance characteristics.
status: active
updated: 2026-10-24
type: Architecture
related:
  - architecture/dicom-streaming.md
---

# DICOM Streaming Use Cases and Performance

Example DICOM streaming pipelines and their performance characteristics. This page is part of [DICOM Streaming Architecture](dicom-streaming.md).

## Use Cases

### 1. Progressive Study Retrieval

**Scenario**: Retrieve 500-instance CT study from remote PACS

**Traditional Approach**: 
- Buffer entire 1GB study in memory
- Wait 30+ seconds before first instance available
- High memory usage, long latency

**Streaming Approach**:
```toml
[pipelines.progressive_retrieve]
endpoints = ["http_api"]
backends = ["remote_pacs_streaming"]
middleware = []
```

**Benefits**:
- First instance available in <100ms
- Constant 2MB memory per instance
- Total memory: ~10MB (5 instances in flight)

### 2. Cross-Site Study Transfer

**Scenario**: Transfer studies between hospitals over WAN

```
Hospital A → Proxy A → HTTP/3 → Proxy B → Hospital B
```

**Proxy A Configuration**:
```toml
[endpoints.dicom_scp_a]
service = "dicom_scp"
local_aet = "HOSPITAL_A"

[backends.local_pacs_a]
service = "dicom_scu_streaming"

[endpoints.http_stream_a]
service = "http_streaming"
format = "ndjson"

[middleware.anonymize_outbound]
type = "transform"
spec_path = "transforms/anonymize-phi.json"
apply = "right"
```

**Proxy B Configuration**:
```toml
[endpoints.http_api_b]
service = "http"

[backends.stream_from_a]
service = "http_streaming_backend"
base_url = "https://proxy-a.hospital-a.org:8443"

[backends.local_pacs_b]
service = "dicom_scu"

[middleware.restore_metadata]
type = "transform"
spec_path = "transforms/restore-local-ids.json"
apply = "right"
```

**Benefits**:
- Secure HTTP/3 transport (TLS 1.3)
- Per-instance PHI anonymization
- Progressive transfer (no study buffering)
- Middleware processing during transfer

### 3. Real-Time Study Processing

**Scenario**: AI inference on incoming DICOM instances

```toml
[pipelines.ai_inference]
endpoints = ["dicom_scp"]
backends = ["ai_backend_streaming"]
middleware = ["preprocess", "ai_transform"]

[middleware.preprocess]
type = "transform"
spec_path = "transforms/normalize-pixels.json"
apply = "right"

[middleware.ai_transform]
type = "transform"
spec_path = "transforms/add-inference-results.json"
apply = "right"
```

**Flow**:
1. Instance received via C-STORE
2. Pixel normalization applied
3. AI inference (external service)
4. Results appended to DICOM metadata
5. Streamed to destination PACS

## Performance

### Benchmarks

**Environment**: 
- 2023 MacBook Pro M3 Max, 64GB RAM
- Local Orthanc PACS
- 500-instance CT study, 2MB avg per instance

**Traditional (Buffered)**:
- Memory: 1.2GB peak
- Latency: 28s until first instance
- Throughput: ~40 instances/sec

**Streaming**:
- Memory: 12MB peak (6 instances in flight)
- Latency: 80ms until first instance
- Throughput: ~120 instances/sec

**Streaming + Middleware**:
- Memory: 15MB peak
- Latency: 95ms until first instance
- Throughput: ~100 instances/sec

### Memory Characteristics

```
Memory Usage Over Time (500 instances)

Traditional:
    1.2GB ┤     ╭────────╮
          │     │        │
    600MB ┤     │        │
          │     │        │
      0MB ┼─────╯        ╰─────
          └──────────────────────▶ Time (30s)

Streaming:
     15MB ┤ ╭─╮ ╭─╮ ╭─╮ ╭─╮ ╭─╮
          │ │ │ │ │ │ │ │ │ │ │
     10MB ┤ │ │ │ │ │ │ │ │ │ │
          │ │ │ │ │ │ │ │ │ │ │
      0MB ┼─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─▶ Time (4s)
          └──────────────────────▶
```

### Tuning

**Concurrency Control** (TODO):
```toml
[backends.pacs_streaming.options]
max_concurrent_instances = 5  # Default: 10
instance_buffer_size = 1      # Default: 1 (no buffering)
```

**Backpressure Handling**:
- `dicom_scu_streaming` C-GET: automatic. The backend answers each C-STORE
  sub-operation only after the instance has been accepted downstream, and holds
  at most 8 instances ahead of the consumer. A slow client therefore slows the
  PACS instead of growing memory on the proxy.
- `http_streaming_backend`: the HTTP response body is read as the consumer
  pulls, so a slow client applies TCP backpressure to the upstream proxy.
